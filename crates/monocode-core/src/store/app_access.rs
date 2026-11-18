use super::{Session, Store};
use serde_json::{json, Value};
use std::collections::HashMap;

/// When a session was last archived and unarchived, in Unix milliseconds. Kept
/// as a small setting per session, so a database written before it existed
/// simply has none: both fields default to unknown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ArchiveTimes {
    pub archived_at: Option<u64>,
    pub unarchived_at: Option<u64>,
}

impl ArchiveTimes {
    /// `archivedAt` of the app API: null unless archived. A session archived
    /// before times were recorded has none, so its last update stands in (the
    /// archive can only be later); null when even that is unknown.
    pub fn archived_at(&self, session: &Session) -> Option<u64> {
        session.archived.then(|| {
            self.archived_at
                .or_else(|| (session.updated_at > 0).then_some(session.updated_at))
        })?
    }
    /// `lastActivityAt` of the app API: the newest of the last message and the
    /// last unarchive; null when neither is known.
    pub fn last_activity_at(&self, session: &Session) -> Option<u64> {
        let at = session.updated_at.max(self.unarchived_at.unwrap_or(0));
        (at > 0).then_some(at)
    }
}

impl Store {
    fn archive_times_key(id: i64) -> String {
        format!("archive-times:{id}")
    }
    pub fn archive_times(&self, id: i64) -> ArchiveTimes {
        self.setting(&Self::archive_times_key(id))
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }
    pub(super) fn record_archive_time(&self, id: i64, archived: bool) -> Result<(), String> {
        let mut times = self.archive_times(id);
        let now = crate::response::now_ms();
        if archived {
            times.archived_at = Some(now);
        } else {
            times.archived_at = None;
            times.unarchived_at = Some(now);
        }
        self.save_setting(
            &Self::archive_times_key(id),
            &serde_json::to_string(&times).map_err(|e| e.to_string())?,
        )
    }
    /// The fields `sessions.list` and `sessions.read` add per session. Git is
    /// only asked (`git`) for a single session, never for a whole list.
    pub fn app_session_facts(
        &self,
        sessions: &[&Session],
        git: bool,
    ) -> Result<HashMap<i64, Value>, String> {
        let dirs: HashMap<i64, String> = self.working_dirs()?.into_iter().collect();
        Ok(sessions
            .iter()
            .map(|session| {
                let times = self.archive_times(session.id);
                let path = dirs
                    .get(&session.id)
                    .and_then(|dir| crate::session_worktree::recorded_worktree(&session.cwd, dir));
                let (dirty, unpushed) = match (&path, git) {
                    (Some(path), true) => {
                        crate::session_worktree::inspect(std::path::Path::new(path))
                    }
                    _ => (None, None),
                };
                (
                    session.id,
                    json!({
                        "archivedAt": times.archived_at(session),
                        "lastActivityAt": times.last_activity_at(session),
                        "worktreePath": path,
                        "worktreeDirty": dirty,
                        "worktreeUnpushed": unpushed,
                    }),
                )
            })
            .collect())
    }

    /// A launch is recoverable as an unsent draft if the app exits before its
    /// provider accepts it. Queue delivery uses this same stable message ID.
    pub fn prepare_app_prompt(&self, id: i64, request: &str, prompt: &str) -> Result<(), String> {
        let mut state = self.follow_ups(id)?;
        let message_id = format!("app-prompt-{}", crate::mono::files::content_hash(request));
        if state.draft.is_none() && !state.messages.iter().any(|m| m.id == message_id) {
            state.revision += 1;
            state.draft = Some(crate::follow_up::Message {
                id: message_id,
                text: prompt.into(),
                ..Default::default()
            });
            self.save_follow_ups(id, &state)?;
        }
        Ok(())
    }
    /// Reserve a request's child durably before its UI/provider is installed.
    /// Shared stores use a deterministic desktop ID; legacy stores reserve their
    /// numeric ID and mapping in the same SQLite transaction.
    pub fn create_app_session(
        &self,
        caller: i64,
        request: &str,
        fingerprint: &str,
        provider: &str,
        project: &str,
    ) -> Result<(Session, bool), String> {
        let key = format!(
            "operator-app-launch:{caller}:{}",
            crate::mono::files::content_hash(request)
        );
        let previous = self
            .setting(&key)?
            .map(|raw| serde_json::from_str::<Value>(&raw).map_err(|e| e.to_string()))
            .transpose()?;
        if previous
            .as_ref()
            .is_some_and(|v| v["fingerprint"] != fingerprint)
        {
            return Err("Request ID was already used with different input".into());
        }
        if let Some(id) = previous.as_ref().and_then(|v| v["sessionId"].as_i64()) {
            return self
                .sessions()?
                .into_iter()
                .find(|s| s.id == id)
                .map(|s| (s, false))
                .ok_or("The session created by this request was removed".into());
        }
        if self.shared.is_some() {
            self.save_setting(&key, &json!({"fingerprint":fingerprint}).to_string())?;
            let original = format!(
                "operator-{}",
                crate::mono::files::content_hash(&format!("{caller}:{request}"))
            );
            let result = self.shared_create_named(provider, project, &original)?;
            self.save_setting(
                &key,
                &json!({"fingerprint":fingerprint,"sessionId":result.0.id}).to_string(),
            )?;
            return Ok(result);
        }
        let transaction = self
            .conn
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let session = self.create(provider, project)?;
        self.save_setting(
            &key,
            &json!({"fingerprint":fingerprint,"sessionId":session.id}).to_string(),
        )?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok((session, true))
    }

    /// The latest exchange ID as `app_conversation` exposes it.
    fn app_latest_turn_id(&self, id: i64, rows: &[crate::transcript::Row]) -> Option<String> {
        if self.shared.is_some() && self.remote_session(id).ok()?.is_none() {
            return self
                .shared_blocks(id)
                .ok()?
                .iter()
                .rfind(|block| crate::app_access::readable_prompt(block))?
                .get("id")?
                .as_str()
                .map(str::to_owned);
        }
        rows.iter()
            .enumerate()
            .rfind(|(index, row)| row.role == "user" && (row.first || *index == 0))
            .map(|(index, _)| format!("row-{index}"))
    }

    pub fn app_completion_report(
        &self,
        id: i64,
        project: Option<&str>,
        count: usize,
    ) -> Result<Option<String>, String> {
        use crate::response::{completion_report, read_call, turn_end};

        // The desktop can append from a different connection. Read the rows,
        // receipt and paging cursor from the same SQLite snapshot.
        let remote = self.remote_session(id)?.is_some();
        let snapshot = (self.shared.is_some() && self.conn.is_autocommit() && !remote)
            .then(|| self.conn.unchecked_transaction())
            .transpose()
            .map_err(|e| e.to_string())?;
        if !remote
            && (id < 0
                || self.persisted_session_id(id).is_err()
                || self.mono_session(id)?.is_none())
        {
            return Ok(None);
        }
        let rows = self.load(id)?.rows;
        let turns = self.agent_settings(id)?.unwrap_or_default().turns;
        let read = project.and_then(|project| {
            let turn = self.app_latest_turn_id(id, &rows)?;
            read_call(id, Some(project), &turn)
        });
        if let Some(snapshot) = snapshot {
            snapshot.commit().map_err(|e| e.to_string())?;
        }
        Ok(Some(completion_report(
            &rows,
            turn_end(&rows, &turns),
            read.as_ref(),
            count,
        )))
    }

    pub fn app_conversation(
        &self,
        id: i64,
        input: &Value,
        busy: bool,
        has_draft: bool,
    ) -> Result<Value, String> {
        // A remote (cloud) session has no row in the local or shared session table: its
        // transcript lives on its host and `load` reads it from there.
        let remote = self.remote_session(id)?.is_some();
        let blocks = if self.shared.is_some() && !remote {
            self.shared_blocks(id)?
        } else {
            let mut blocks: Vec<Value> = Vec::new();
            for (i, row) in self.load(id)?.rows.into_iter().enumerate() {
                if !row.first && !blocks.is_empty() {
                    let block = blocks.last_mut().unwrap();
                    let text = block["text"].as_str().unwrap_or("").to_owned() + &row.text;
                    block["text"] = json!(text);
                } else {
                    blocks.push(json!({"id":format!("row-{i}"),"role":row.role,"text":row.text}));
                }
            }
            blocks
        };
        let mut page = crate::app_access::conversation_page(&blocks, input)?;
        page["sessionId"] = json!(id);
        page["busy"] = json!(busy);
        page["hasDraft"] = json!(has_draft || blocks.iter().any(|b| b["draft"] == true));
        if let Some(session) = self.sessions()?.into_iter().find(|s| s.id == id) {
            page["title"] = json!(session.title);
            if let Some(Value::Object(facts)) =
                self.app_session_facts(&[&session], true)?.remove(&id)
            {
                for (key, value) in facts {
                    page[key] = value;
                }
            }
        }
        Ok(page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::response::{completion_report, read_call, turn_end, TurnEnd};
    use std::path::Path;

    /// The report reads projected rows, where a plan or an unstructured task
    /// list shows as assistant prose; `sessions.read` reads the canonical
    /// blocks. Both must pick the same message, so the read call returns it.
    #[test]
    fn report_and_read_call_agree_on_projected_plan_and_task_messages() {
        let long = format!("{}PLAN END", "step\n".repeat(4000));
        for (trailing, text) in [
            (
                json!({"id":"p","role":"plan","text":"# Plan\n\n- Fix it"}),
                "# Plan\n\n- Fix it",
            ),
            (
                json!({"id":"t","role":"tasks","text":"1. Fix it"}),
                "1. Fix it",
            ),
            (json!({"id":"p","role":"plan","text":long}), long.as_str()),
        ] {
            let store = Store::open_monocode(Path::new(":memory:")).unwrap();
            let session = store.create("codex", "/work/repo").unwrap();
            store.provider_id(session.id, "thread").unwrap();
            let blocks = vec![
                json!({"id":"u","role":"user","text":"Plan the fix"}),
                json!({"id":"a","role":"assistant","text":"THE ANSWER"}),
                trailing,
            ];
            store.shared_store_blocks(session.id, &blocks).unwrap();
            let rows = store.load(session.id).unwrap().rows;
            let turn = store.app_latest_turn_id(session.id, &rows).unwrap();
            let call = read_call(session.id, Some("/work/repo"), &turn).unwrap();
            let report = completion_report(&rows, TurnEnd::Unknown, Some(&call), 12000);
            let expected = crate::response::normalized(text);
            let mut input = call.clone();
            let mut read = String::new();
            loop {
                let page = store
                    .app_conversation(session.id, &input, false, false)
                    .unwrap();
                let message = &page["turns"][0]["assistant"];
                read.push_str(message["text"].as_str().unwrap());
                match message.get("nextCharOffset") {
                    Some(next) => input["charOffset"] = next.clone(),
                    None => break,
                }
            }
            assert_eq!(read, expected, "sessions.read returns the reported message");
            assert!(expected.ends_with(report.rsplit("\n\n").next().unwrap()));
            if expected.chars().count() <= 12000 {
                assert_eq!(report, expected);
            }
        }
    }

    #[test]
    fn shared_settings_include_the_persisted_stop_marker() {
        let store = Store::open_monocode(Path::new(":memory:")).unwrap();
        let id = store.create("codex", "/work/repo").unwrap().id;
        store
            .shared_store_blocks(
                id,
                &[json!({"id":"u","role":"user","text":"Go","nativeStopped":true})],
            )
            .unwrap();
        assert!(store.agent_settings(id).unwrap().unwrap().turns[&1].stopped);
    }

    #[test]
    fn a_receipt_read_error_is_distinct_from_a_removed_session() {
        let mut store = Store::open(Path::new(":memory:")).unwrap();
        let id = store.create("codex", "/work/repo").unwrap().id;
        store.save_setting(&format!("agent:{id}"), "{").unwrap();
        assert!(store.app_completion_report(id, None, 12000).is_err());
        store.delete(id).unwrap();
        assert_eq!(store.app_completion_report(id, None, 12000).unwrap(), None);
    }

    /// A native stop leaves no row. The receipt the runtime closes records it,
    /// so the report still says the turn was stopped after a restart.
    #[test]
    fn a_stopped_turn_still_reports_unfinished_after_reopening_the_store() {
        let dir = std::env::temp_dir().join(format!(
            "monocode-stopped-{}-{}",
            std::process::id(),
            crate::time::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let progress = "I will investigate this now";
        for shared in [false, true] {
            let path = dir.join(format!("store-{shared}.sqlite"));
            let open = |path: &Path| {
                if shared {
                    Store::open_monocode(path).unwrap()
                } else {
                    Store::open(path).unwrap()
                }
            };
            let id = {
                let mut store = open(&path);
                let session = store.create("codex", "/work/repo").unwrap();
                let mut transcript = crate::transcript::Transcript::default();
                for (role, text) in [("user", "Investigate"), ("assistant", progress)] {
                    transcript.begin(role);
                    transcript.append(text);
                }
                store.write_rows(session.id, 0, &transcript.rows).unwrap();
                let mut settings = store
                    .agent_settings(session.id)
                    .unwrap()
                    .unwrap_or_default();
                settings.turns.insert(
                    1,
                    crate::agent::TurnInfo {
                        started_at: Some(1),
                        duration_known: true,
                        stopped: true,
                        ..Default::default()
                    },
                );
                store.save_agent_settings(session.id, &settings).unwrap();
                session.id
            };
            let store = open(&path);
            let rows = store.load(id).unwrap().rows;
            let turns = store.agent_settings(id).unwrap().unwrap_or_default().turns;
            assert_eq!(
                turn_end(&rows, &turns),
                TurnEnd::Stopped,
                "shared: {shared}"
            );
            assert_eq!(
                completion_report(&rows, turn_end(&rows, &turns), None, 12000),
                format!("(its latest turn did not finish: it was stopped. This is the last message it wrote, not a final answer:)\n\n{progress}")
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
