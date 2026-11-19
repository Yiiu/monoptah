//! Claude `/goal` lifecycle notices: parse the CLI's synthetic messages into
//! typed state. Version-coupled to the claude CLI (verified 2.1.311/2.1.325);
//! the notice→phase table lives only here.
use super::{Kind, Request};
use crate::agent::Settings;
use crate::transcript::{GoalNotice, GoalPhase, Row};
use serde_json::Value;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Recover a completion that predates the stream fix. A dated saved goal and
/// a newer matching attachment are required, so an identical older goal is
/// never mistaken for this one.
pub fn recover_claude_goal(
    cwd: &Path,
    session: &str,
    settings: &Settings,
    rows: &[Row],
) -> Option<GoalNotice> {
    let (start, _, condition) = GoalNotice::active_message(rows)?;
    if condition.is_empty() {
        return None;
    }
    let receipt = rows[start].sent_at?;
    let since = GoalNotice::command_time(&rows[..start], &condition)
        .map_or(receipt, |submitted| receipt.min(submitted));
    let mut cursor = TranscriptCursor::for_session(cwd, session, settings)?;
    cursor.condition = Some(condition);
    cursor.find_completion(since)
}

/// Local stream-json drops `goal_status` attachments (observed in Claude Code
/// 2.1.327). Read the same provider session's JSONL at the result boundary.
/// The starting offset keeps a resumed turn from replaying old completions.
pub(super) struct TranscriptCursor {
    projects: PathBuf,
    path: PathBuf,
    session: String,
    offset: u64,
    started_at: u64,
    condition: Option<String>,
    terminal_seen: bool,
    partial_line: bool,
}

impl TranscriptCursor {
    pub(super) fn before_prompt(request: &Request) -> Option<Self> {
        let mut cursor = Self::for_session(
            &request.cwd,
            request.provider_id.as_deref()?,
            &request.settings,
        )?;
        match cursor.open() {
            Ok(file) => cursor.offset = file.metadata().ok()?.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return None,
        }
        cursor.started_at = crate::time::now_ms();
        cursor
            .condition
            .clone_from(&request.settings.claude_goal_condition);
        Some(cursor)
    }

    fn for_session(cwd: &Path, session: &str, settings: &Settings) -> Option<Self> {
        if uuid::Uuid::parse_str(session).ok()?.to_string() != session {
            return None;
        }
        let home = settings
            .account
            .directory(Kind::Claude)
            .ok()?
            .or_else(|| {
                settings
                    .environment
                    .get("CLAUDE_CONFIG_DIR")
                    .filter(|dir| !dir.is_empty())
                    .map(PathBuf::from)
            })
            .or_else(|| settings.account.home(Kind::Claude).ok())?;
        let home = if home.is_relative() {
            cwd.join(home)
        } else {
            home
        };
        let projects = home.join("projects");
        let project = crate::cloud_sessions::claude_project_dir_name(&cwd.to_string_lossy());
        Some(Self {
            path: projects.join(project).join(format!("{session}.jsonl")),
            projects,
            session: session.into(),
            offset: 0,
            started_at: 0,
            condition: None,
            terminal_seen: false,
            partial_line: false,
        })
    }

    pub(super) fn observe_notice(&mut self, notice: &GoalNotice) {
        match notice.phase {
            GoalPhase::Set => {
                self.condition.clone_from(&notice.condition);
                self.terminal_seen = false;
            }
            GoalPhase::Cleared | GoalPhase::Achieved | GoalPhase::Error => {
                self.terminal_seen = true;
            }
            GoalPhase::Checkin | GoalPhase::Paused => {}
        }
    }

    fn open(&mut self) -> std::io::Result<File> {
        match File::open(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Claude may resolve the cwd differently or resume a transcript
                // stored under an older project key. Accept only a unique match.
                let name = format!("{}.jsonl", self.session);
                let mut matches = fs::read_dir(&self.projects)?
                    .flatten()
                    .map(|entry| entry.path().join(&name))
                    .filter(|path| path.is_file());
                let path = matches
                    .next()
                    .filter(|_| matches.next().is_none())
                    .ok_or(error)?;
                let file = File::open(&path)?;
                self.path = path;
                Ok(file)
            }
            result => result,
        }
    }

    pub(super) fn completion(&mut self, session: &str) -> Option<GoalNotice> {
        self.completion_with_retry(session, || {
            std::thread::sleep(std::time::Duration::from_millis(100))
        })
    }

    fn completion_with_retry(&mut self, session: &str, wait: impl FnOnce()) -> Option<GoalNotice> {
        if session != self.session || self.terminal_seen {
            return None;
        }
        let first = self.find_completion(self.started_at);
        if first.is_some() || (self.condition.is_none() && !self.partial_line) {
            return first;
        }
        // Claude can flush the attachment just after the successful result.
        // Re-read from the last complete line on this provider worker only.
        wait();
        self.find_completion(self.started_at)
    }

    fn find_completion(&mut self, since: u64) -> Option<GoalNotice> {
        let mut reader = BufReader::new(self.open().ok()?);
        reader.seek(SeekFrom::Start(self.offset)).ok()?;
        let mut line = Vec::new();
        let mut latest = None;
        loop {
            line.clear();
            let bytes = reader.read_until(b'\n', &mut line).ok()?;
            if line.last() != Some(&b'\n') {
                self.partial_line = bytes > 0;
                break;
            }
            self.offset += bytes as u64;
            let Ok(text) = std::str::from_utf8(&line) else {
                continue;
            };
            if !text.contains("\"goal_status\"") {
                continue;
            }
            let Ok(row) = serde_json::from_str::<Value>(text) else {
                continue;
            };
            let status = &row["attachment"];
            if row["type"] == "attachment"
                && row["sessionId"] == self.session.as_str()
                && row["isSidechain"] == false
                && status["type"] == "goal_status"
                && row["timestamp"]
                    .as_str()
                    .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
                    .and_then(|time| u64::try_from(time.timestamp_millis()).ok())
                    .is_some_and(|time| time >= since)
            {
                // A new unmet goal supersedes an older met goal in the batch.
                latest = if status["met"] == true {
                    status["condition"]
                        .as_str()
                        .map(str::trim)
                        .map(str::to_owned)
                } else {
                    None
                };
            }
        }
        let latest = latest?;
        if self.terminal_seen
            || self
                .condition
                .as_ref()
                .is_some_and(|current| *current != latest)
        {
            return None;
        }
        self.terminal_seen = true;
        Some(GoalNotice {
            phase: GoalPhase::Achieved,
            condition: Some(latest),
            notice: "Goal achieved".into(),
        })
    }
}

/// Map a synthetic notice line to a goal state change, if it is one.
pub fn goal_notice(text: &str) -> Option<crate::transcript::GoalNotice> {
    let line = text.trim();
    let (phase, condition) = if let Some(rest) = line.strip_prefix("Goal set: ") {
        (GoalPhase::Set, Some(rest.trim().to_owned()))
    } else if line == "No goal set" {
        (GoalPhase::Cleared, None)
    } else if line.starts_with("Goal cleared after an unrecoverable error") {
        (GoalPhase::Error, None)
    } else if let Some(rest) = line.strip_prefix("Goal cleared: ") {
        (GoalPhase::Cleared, Some(rest.trim().to_owned()))
    } else if line.starts_with("Goal achieved") {
        (GoalPhase::Achieved, None)
    } else if line.starts_with("Goal still active") {
        (GoalPhase::Checkin, None)
    } else if line.starts_with("Goal paused") || line.starts_with("Goal could not be achieved") {
        (GoalPhase::Paused, None)
    } else {
        return None;
    };
    Some(crate::transcript::GoalNotice {
        phase,
        condition,
        notice: line.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Event, Kind, Protocol, Request};
    use crate::transcript::GoalNotice;
    use serde_json::{json, Value};
    use std::{fs, io::Write, path::PathBuf};

    const SESSION: &str = "cf9168df-2021-4ef4-9d0c-1a0368d520e7";

    struct Turn {
        protocol: Protocol,
        transcript: PathBuf,
        _config: tempfile::TempDir,
    }

    impl Turn {
        fn new() -> Self {
            let config = tempfile::tempdir().unwrap();
            let cwd = config.path().join("worktree");
            fs::create_dir(&cwd).unwrap();
            let project = crate::cloud_sessions::claude_project_dir_name(&cwd.to_string_lossy());
            let project = config.path().join("projects").join(project);
            fs::create_dir_all(&project).unwrap();
            let mut request = Request {
                kind: Kind::Claude,
                binary: "unused".into(),
                cwd,
                prompt: "Continue the work".into(),
                provider_id: Some(SESSION.into()),
                model: None,
                settings: Default::default(),
            };
            request.settings.environment.insert(
                "CLAUDE_CONFIG_DIR".into(),
                config.path().to_str().unwrap().into(),
            );
            Self {
                protocol: Protocol::new(request),
                transcript: project.join(format!("{SESSION}.jsonl")),
                _config: config,
            }
        }

        fn status(&self, met: bool, condition: &str) {
            self.status_at(met, condition, &chrono::Utc::now().to_rfc3339());
        }

        fn status_at(&self, met: bool, condition: &str, timestamp: &str) {
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.transcript)
                .unwrap();
            writeln!(
                file,
                "{}",
                json!({"type":"attachment", "sessionId":SESSION, "timestamp":timestamp,
                "isSidechain":false, "attachment":{"type":"goal_status", "met":met,
                "condition":condition}})
            )
            .unwrap();
        }

        fn frame(&mut self, frame: Value) -> Vec<Event> {
            self.protocol.frame(frame, &mut Vec::new()).unwrap()
        }

        fn start(&mut self) {
            self.frame(json!({"type":"control_response", "response":{"request_id":"native-init"}}));
        }

        fn notice(&mut self, text: &str) -> Vec<Event> {
            self.frame(json!({"type":"assistant", "session_id":SESSION, "message":{
                "model":"<synthetic>", "content":[{"type":"text", "text":text}]}}))
        }

        fn result(&mut self) -> Vec<Event> {
            self.frame(
                json!({"type":"result", "subtype":"success", "session_id":SESSION,
                "is_error":false, "result":"Goal achieved"}),
            )
        }
    }

    fn achieved(events: &[Event]) -> bool {
        events.iter().any(
            |event| matches!(event, Event::Goal(notice) if notice.phase == GoalPhase::Achieved),
        )
    }

    #[test]
    fn successful_result_recovers_completion_missing_from_the_wire() {
        let mut turn = Turn::new();
        turn.start();
        turn.status(false, "fix the steering lag");
        turn.notice("Goal set: fix the steering lag");
        turn.frame(json!({"type":"assistant", "session_id":SESSION, "message":{
            "model":"model", "stop_reason":"end_turn",
            "content":[{"type":"text", "text":"The steering tests pass."}]}}));
        turn.status(true, "fix the steering lag");
        let events = turn.result();
        assert!(
            achieved(&events),
            "the disk-only completion must reach the UI"
        );
        assert_eq!(events.last(), Some(&Event::Done));
        assert!(!achieved(&turn.result()), "the attachment is consumed once");
    }

    #[test]
    fn a_resumed_turn_accepts_only_new_completion_evidence() {
        let mut resumed = Turn::new();
        resumed.status(true, "an older goal");
        resumed.start();
        assert!(!achieved(&resumed.result()));

        let mut resumed = Turn::new();
        resumed.status(false, "finish the resumed goal");
        resumed.start();
        resumed.status(true, "finish the resumed goal");
        assert!(achieved(&resumed.result()));
    }

    #[test]
    fn resumed_turn_ignores_a_replayed_old_completion() {
        let mut turn = Turn::new();
        turn.status(false, "new goal");
        turn.start();
        turn.status_at(true, "old goal", "2000-01-01T00:00:00Z");
        assert!(!achieved(&turn.result()));
    }

    #[test]
    fn newer_unmet_goal_wins_over_an_earlier_completion() {
        let mut turn = Turn::new();
        turn.start();
        turn.status(true, "previous goal");
        turn.status(false, "new goal");
        turn.notice("Goal set: new goal");
        assert!(!achieved(&turn.result()));
    }

    #[test]
    fn synthetic_completion_is_not_reported_twice() {
        let mut turn = Turn::new();
        turn.start();
        turn.status(true, "current goal");
        assert!(achieved(&turn.notice("Goal achieved")));
        assert!(!achieved(&turn.result()));
    }

    #[test]
    fn reopened_goal_recovers_only_completion_after_its_saved_notice() {
        let turn = Turn::new();
        fs::write(
            &turn.transcript,
            format!(
                "{}\n",
                json!({"type":"attachment",
            "timestamp":"2026-11-18T08:10:39.628Z", "sessionId":SESSION, "isSidechain":false,
            "attachment":{"type":"goal_status","met":true,"condition":"finish it"}})
            ),
        )
        .unwrap();
        let mut saved = Row {
            role: "goal".into(),
            text: serde_json::to_string(&goal_notice("Goal set: finish it").unwrap()).unwrap(),
            first: true,
            sent_at: Some(1794988000000),
        };
        let recover = |row: &Row| {
            recover_claude_goal(
                &turn.protocol.request.cwd,
                SESSION,
                &turn.protocol.request.settings,
                std::slice::from_ref(row),
            )
        };
        assert_eq!(recover(&saved).unwrap().phase, GoalPhase::Achieved);
        saved.sent_at = Some(1794989500000);
        assert!(
            recover(&saved).is_none(),
            "an older attachment cannot close the replacement goal"
        );
        saved.sent_at = Some(1794988000000);
        saved.text = serde_json::to_string(&goal_notice("No goal set").unwrap()).unwrap();
        assert!(
            recover(&saved).is_none(),
            "an already closed goal needs no second row"
        );
    }

    #[test]
    fn result_retries_a_partially_flushed_goal_attachment() {
        let mut turn = Turn::new();
        turn.start();
        turn.status(true, "finish");
        let file = fs::OpenOptions::new()
            .write(true)
            .open(&turn.transcript)
            .unwrap();
        file.set_len(file.metadata().unwrap().len() - 1).unwrap();
        drop(file);
        let cursor = turn.protocol.claude_goal.as_mut().unwrap();
        let completed = cursor
            .completion_with_retry(SESSION, || {
                fs::OpenOptions::new()
                    .append(true)
                    .open(&turn.transcript)
                    .unwrap()
                    .write_all(b"\n")
                    .unwrap();
            })
            .unwrap();
        assert_eq!(completed.phase, GoalPhase::Achieved);
        assert!(!achieved(&turn.result()));
    }

    #[test]
    fn resumed_active_goal_retries_when_the_whole_attachment_is_late() {
        let mut turn = Turn::new();
        turn.protocol.request.settings.claude_goal_condition = Some("finish".into());
        turn.start();
        let mut cursor = turn.protocol.claude_goal.take().unwrap();
        let completed = cursor
            .completion_with_retry(SESSION, || turn.status(true, "finish"))
            .unwrap();
        assert_eq!(completed.phase, GoalPhase::Achieved);
    }

    #[test]
    fn reopened_split_goal_uses_the_command_echo_submission_time() {
        let turn = Turn::new();
        let condition = "修".repeat(400);
        let mut t = crate::transcript::Transcript::default();
        let i = t.begin("interjection");
        t.rows[i].sent_at = Some(1_000);
        t.append(
            &json!({"custom_type":"command", "text":format!("/goal {condition}")}).to_string(),
        );
        let i = t.begin("goal");
        t.rows[i].sent_at = Some(3_000);
        t.append(
            &serde_json::to_string(&goal_notice(&format!("Goal set: {condition}")).unwrap())
                .unwrap(),
        );
        assert!(t.rows.len() > i + 1);
        turn.status_at(true, &condition, "1970-01-01T00:00:02Z");
        let recover = |rows: &[Row]| {
            recover_claude_goal(
                &turn.protocol.request.cwd,
                SESSION,
                &turn.protocol.request.settings,
                rows,
            )
        };
        assert_eq!(
            GoalNotice::active(&t.rows),
            Some((GoalPhase::Set, condition.clone()))
        );
        assert_eq!(recover(&t.rows).unwrap().phase, GoalPhase::Achieved);
        t.rows[0].text = json!({"custom_type":"command", "text":"/goal other"}).to_string();
        assert!(recover(&t.rows).is_none());
    }

    #[test]
    fn missing_transcript_does_not_block_the_result() {
        let mut turn = Turn::new();
        turn.start();
        let events = turn.result();
        assert!(!achieved(&events));
        assert_eq!(events.last(), Some(&Event::Done));
    }

    #[test]
    fn parses_every_notice_in_the_transition_table() {
        let set = goal_notice("Goal set: reply with ok").unwrap();
        assert_eq!(set.phase, GoalPhase::Set);
        assert_eq!(set.condition.as_deref(), Some("reply with ok"));
        assert_eq!(
            goal_notice("No goal set").map(|g| g.phase),
            Some(GoalPhase::Cleared)
        );
        assert_eq!(
            goal_notice("Goal cleared: reply with ok").map(|g| g.phase),
            Some(GoalPhase::Cleared)
        );
        assert_eq!(
            goal_notice("Goal achieved").map(|g| g.phase),
            Some(GoalPhase::Achieved)
        );
        assert_eq!(
            goal_notice("Goal still active · 2 · retrying in 4s").map(|g| g.phase),
            Some(GoalPhase::Checkin)
        );
        assert_eq!(
            goal_notice("Goal paused · the last turn could not finish · x").map(|g| g.phase),
            Some(GoalPhase::Paused)
        );
        assert_eq!(
            goal_notice("Goal could not be achieved").map(|g| g.phase),
            Some(GoalPhase::Paused)
        );
        let err = goal_notice(
            "Goal cleared after an unrecoverable error (2): boom. Run /goal again to continue.",
        )
        .unwrap();
        assert_eq!(err.phase, GoalPhase::Error);
    }

    #[test]
    fn cleared_notices_carry_the_condition() {
        let cleared = goal_notice("Goal cleared: fix the login flow").unwrap();
        assert_eq!(cleared.phase, GoalPhase::Cleared);
        assert_eq!(cleared.condition.as_deref(), Some("fix the login flow"));
    }

    #[test]
    fn unknown_text_is_not_a_goal_notice() {
        assert!(goal_notice("Here is your summary").is_none());
        assert!(goal_notice("").is_none());
        // Informational only: no state change.
        assert!(goal_notice("Goal condition is limited to 500 characters").is_none());
    }

    #[test]
    fn notice_keeps_the_full_line() {
        let line = "Goal set: ship the release";
        let notice: GoalNotice = goal_notice(line).unwrap();
        assert_eq!(notice.notice, line);
    }
}
