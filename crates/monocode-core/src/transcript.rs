//! Incremental, UTF-8-safe transcript blocks. Even a giant single answer is virtualizable.
use serde::{Deserialize, Serialize};

pub mod attachments;

pub const ROW_BYTES: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interjection {
    pub text: String,
    pub custom_type: String,
    pub severity: Option<String>,
}
impl Interjection {
    pub fn label(&self) -> &str {
        match self.custom_type.as_str() {
            "advisor" => "Advisor",
            "custom" => "Notice",
            "hook" => "Hook",
            label => label,
        }
    }
}

/// A claude `/goal` lifecycle notice parsed off the wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalNotice {
    pub phase: GoalPhase,
    /// The goal's condition, when the notice carries one (set/cleared).
    pub condition: Option<String>,
    /// The full notice line, so naive renderers still show something.
    pub notice: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GoalPhase {
    Set,
    Checkin,
    Paused,
    Cleared,
    Achieved,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub role: String,
    pub text: String,
    pub first: bool,
    /// Wall-clock send time on the first row; older histories may not have one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<u64>,
}
impl Row {
    pub fn is_prompt(&self) -> bool {
        matches!(self.role.as_str(), "user" | "mono-system")
    }
}

/// Whole messages, newest first. A paged transcript may start in the middle
/// of a message; that leading fragment cannot be interpreted on its own.
fn messages_rev(mut rows: &[Row]) -> impl Iterator<Item = (usize, &[Row])> {
    std::iter::from_fn(move || {
        let start = rows.iter().rposition(|row| row.first)?;
        let (older, message) = rows.split_at(start);
        rows = older;
        Some((start, message))
    })
}

fn message_text(message: &[Row]) -> std::borrow::Cow<'_, str> {
    if message.len() == 1 {
        return message[0].text.as_str().into();
    }
    message
        .iter()
        .map(|row| row.text.as_str())
        .collect::<String>()
        .into()
}

impl GoalNotice {
    /// The latest active goal, including the set condition for checkins/pauses.
    pub fn active(rows: &[Row]) -> Option<(GoalPhase, String)> {
        let (_, phase, condition) = Self::active_message(rows)?;
        Some((phase, condition))
    }

    pub(crate) fn active_message(rows: &[Row]) -> Option<(usize, GoalPhase, String)> {
        let mut goals = messages_rev(rows).filter(|(_, msg)| msg[0].role == "goal");
        let (index, message) = goals.next()?;
        let latest: Self = serde_json::from_str(&message_text(message)).ok()?;
        let phase = match latest.phase {
            GoalPhase::Set => return Some((index, GoalPhase::Set, latest.condition?)),
            GoalPhase::Checkin => GoalPhase::Set,
            GoalPhase::Paused => GoalPhase::Paused,
            _ => return None,
        };
        let set = goals
            .filter_map(|(_, msg)| serde_json::from_str::<Self>(&message_text(msg)).ok())
            .find(|notice| notice.phase == GoalPhase::Set);
        Some((
            index,
            phase,
            set.and_then(|notice| notice.condition).unwrap_or_default(),
        ))
    }

    /// Older sessions used user rows; current clients echo raw commands as
    /// interjections. Only the nearest prompt/command before this goal counts.
    pub(crate) fn command_time(rows: &[Row], condition: &str) -> Option<u64> {
        let (command, sent_at) = messages_rev(rows)
            .take_while(|(_, msg)| msg[0].role != "goal")
            .find_map(|(_, msg)| {
                let command = match msg[0].role.as_str() {
                    "user" => message_text(msg),
                    "interjection" => {
                        let echo: Interjection = serde_json::from_str(&message_text(msg)).ok()?;
                        if echo.custom_type != "command" {
                            return None;
                        }
                        echo.text.into()
                    }
                    _ => return None,
                };
                Some((command, msg[0].sent_at))
            })?;
        (command.trim().strip_prefix("/goal ")?.trim() == condition).then_some(sent_at)?
    }
}

#[derive(Default, Debug)]
pub struct Transcript {
    pub rows: Vec<Row>,
}

impl Transcript {
    pub fn begin(&mut self, role: &str) -> usize {
        let ix = self.rows.len();
        self.rows.push(Row {
            role: role.into(),
            text: String::new(),
            first: true,
            sent_at: Some(crate::time::now_ms()),
        });
        ix
    }

    /// Returns the first changed row. Earlier rows retain their identities and content.
    pub fn append(&mut self, mut text: &str) -> usize {
        assert!(!self.rows.is_empty(), "begin a message before appending");
        let changed = self.rows.len() - 1;
        while !text.is_empty() {
            let last = self.rows.last_mut().unwrap();
            let lines = last.text.matches('\n').count();
            let room = if lines >= 12 {
                0
            } else {
                ROW_BYTES - last.text.len()
            };
            let mut end = room.min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            // Bound height as well as bytes, including pathological newline floods.
            if let Some((ix, _)) = text[..end].match_indices('\n').nth(11 - lines.min(11)) {
                end = ix + 1;
            }
            if end == 0 {
                let role = last.role.clone();
                self.rows.push(Row {
                    role,
                    text: String::new(),
                    first: false,
                    sent_at: None,
                });
                continue;
            }
            last.text.push_str(&text[..end]);
            text = &text[end..];
            if !text.is_empty() {
                let role = last.role.clone();
                self.rows.push(Row {
                    role,
                    text: String::new(),
                    first: false,
                    sent_at: None,
                });
            }
        }
        changed
    }
}

/// Port of integrations/harness/core/streamText.ts::snapshotRemainder.
/// Tokens append verbatim. Completed snapshots contribute only missing suffixes.
pub fn snapshot_remainder<'a>(already: &str, snapshot: &'a str) -> &'a str {
    if let Some(suffix) = snapshot.strip_prefix(already) {
        suffix
    } else if already.starts_with(snapshot) {
        ""
    } else {
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn giant_unicode_message_and_newline_flood_are_bounded() {
        let mut transcript = Transcript::default();
        transcript.begin("assistant");
        let source = "🙂e\u{301}漢字\n".repeat(40_000);
        transcript.append(&source);
        assert!(transcript.rows.iter().all(|r| r.text.len() <= ROW_BYTES));
        assert!(transcript
            .rows
            .iter()
            .all(|r| r.text.matches('\n').count() <= 12));
        assert_eq!(
            transcript
                .rows
                .iter()
                .map(|r| r.text.as_str())
                .collect::<String>(),
            source
        );
    }
    #[test]
    fn append_changes_only_tail_and_preserves_blank_lines() {
        let mut t = Transcript::default();
        t.begin("assistant");
        t.append(&"x".repeat(ROW_BYTES));
        let prefix = t.rows[0].clone();
        t.append("\n");
        t.append("\n");
        assert_eq!(t.rows[0], prefix);
        assert_eq!(t.rows[1].text, "\n\n");
        assert_eq!(snapshot_remainder("Hello", "Hello world"), " world");
        assert_eq!(snapshot_remainder("Hello world", "Hello"), "");
        assert_eq!(snapshot_remainder("Hello", "Hello"), "");
    }
}
