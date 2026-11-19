use super::{ClaudeMessageUsage, Usage};
use crate::accounts::Profile;
use crate::cloud_sessions::{claude_project_dir_name, plain_path};
use crate::provider::Kind;
use serde::Deserialize;
use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

impl Usage {
    /// Older clients saved Claude's turn totals as context occupancy. Recover the
    /// latest main request on reopen, including totals that happen to fit the window.
    pub fn restore_local_claude(
        self,
        account: &Profile,
        cwd: &str,
        thread: Option<&str>,
        turn_metrics: Option<crate::agent::TurnMetrics>,
    ) -> Option<Self> {
        let Some(window) = self.window.filter(|&w| w > 0) else {
            return Some(self);
        };
        let turn_total = turn_metrics.map(|m| {
            m.input_tokens
                .saturating_add(m.cache_read_tokens)
                .saturating_add(m.cache_write_tokens)
                .saturating_add(m.output_tokens)
        });
        // A valid per-request reading (possibly from a final iteration) takes
        // precedence over the earlier assistant snapshot in the transcript.
        let keep_saved = self.used > 0 && self.used <= window && Some(self.used) != turn_total;
        let used = thread.and_then(|thread| {
            crate::accounts::transfer::codex::validate_thread_id(thread).ok()?;
            let home = account.home(Kind::Claude).ok()?;
            let cwd = plain_path(cwd.into());
            let project = claude_project_dir_name(cwd.to_str()?);
            latest_used(
                &home
                    .join("projects")
                    .join(project)
                    .join(format!("{thread}.jsonl")),
                thread,
                keep_saved.then_some(self.used),
            )
        });
        match used {
            Some(HistoryReading::Compacted(Some(used))) if used <= window => {
                Some(Self { used, ..self })
            }
            Some(HistoryReading::Used(used)) if used <= window => Some(Self { used, ..self }),
            _ if keep_saved => Some(self),
            Some(HistoryReading::Compacted(_)) => None,
            _ => (self.used > 0 && self.used <= window).then_some(self),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    session_id: Option<&'a str>,
    #[serde(default)]
    is_sidechain: bool,
    #[serde(rename = "parent_tool_use_id")]
    parent: Option<&'a str>,
    subtype: Option<&'a str>,
    compact_metadata: Option<CompactMetadata>,
    #[serde(borrow)]
    message: Option<Message<'a>>,
}

#[derive(Deserialize)]
struct Message<'a> {
    model: Option<&'a str>,
    usage: Option<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompactMetadata {
    post_tokens: Option<u64>,
}

enum HistoryReading {
    Used(u64),
    Compacted(Option<u64>),
}

fn latest_used(path: &Path, thread: &str, saved: Option<u64>) -> Option<HistoryReading> {
    // A transcript can be hundreds of MB. Inspect only its tail and deserialize
    // the fields we need, without materializing tool results or message bodies.
    const TAIL: u64 = 2 * 1024 * 1024;
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut bytes = Vec::new();
    // Most recent readings fit in a few lines. Only expand the read when e.g.
    // a large compact summary or tool result follows the boundary.
    for limit in [16 * 1024, TAIL] {
        let start = len.saturating_sub(limit);
        file.seek(SeekFrom::Start(start)).ok()?;
        bytes.clear();
        (&mut file).take(len - start).read_to_end(&mut bytes).ok()?;
        let mut lines = bytes.split(|&b| b == b'\n');
        if start > 0 {
            lines.next();
        }
        if let Some(reading) = latest_in_lines(lines.rev(), thread, saved) {
            return Some(reading);
        }
        if start == 0 {
            break;
        }
    }
    None
}

fn latest_in_lines<'a>(
    lines: impl Iterator<Item = &'a [u8]>,
    thread: &str,
    saved: Option<u64>,
) -> Option<HistoryReading> {
    let mut latest_request = None;
    for line in lines {
        let Ok(row) = serde_json::from_slice::<Record>(line) else {
            continue;
        };
        if row.session_id != Some(thread) || row.is_sidechain || row.parent.is_some() {
            continue;
        }
        if row.kind == "system" && row.subtype == Some("compact_boundary") {
            let post = row.compact_metadata.and_then(|m| m.post_tokens);
            if let (Some(latest), Some(saved)) = (latest_request, saved) {
                // Only replace a plausible saved reading when it is still the
                // previous boundary's count, not a later final iteration.
                return Some(HistoryReading::Used(if post == Some(saved) {
                    latest
                } else {
                    saved
                }));
            }
            return Some(HistoryReading::Compacted(post));
        }
        if latest_request.is_none() && row.kind == "assistant" {
            if let Some(message) = row.message.filter(|m| m.model != Some("<synthetic>")) {
                let used = ClaudeMessageUsage::default().observe(&message.usage?)?;
                if saved.is_none() || saved == Some(used) {
                    return Some(HistoryReading::Used(used));
                }
                latest_request = Some(used);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn context_restores_the_last_main_request_for_a_saved_windows_session() {
        let root = tempfile::tempdir().unwrap();
        let account = Profile::new("fixture", root.path());
        let project = account
            .home(Kind::Claude)
            .unwrap()
            .join("projects/C--dev-monoptah");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("thread.jsonl"), concat!(
            "{\"type\":\"assistant\",\"sessionId\":\"thread\",\"message\":{\"usage\":{\"input_tokens\":220000}}}\n",
            "{\"type\":\"assistant\",\"sessionId\":\"thread\",\"message\":{\"usage\":{\"input_tokens\":5738,\"cache_read_input_tokens\":40960,\"output_tokens\":196}}}\n",
            "{\"type\":\"assistant\",\"sessionId\":\"thread\",\"isSidechain\":true,\"message\":{\"usage\":{\"input_tokens\":99000}}}\n",
            "{\"type\":\"assistant\",\"sessionId\":\"thread\",\"message\":{\"model\":\"<synthetic>\",\"usage\":{\"output_tokens\":1}}}\n",
            "{\"type\":\"assistant\",\"sessionId\":\"other\",\"message\":{\"usage\":{\"input_tokens\":123}}}\n"
        )).unwrap();
        for old in [1_647_660, 221_355, 0] {
            let saved = Usage {
                used: old,
                window: Some(500_000),
            };
            let metrics = crate::agent::TurnMetrics {
                input_tokens: old,
                ..Default::default()
            };
            let restored = saved
                .restore_local_claude(
                    &account,
                    r"\\?\C:\dev\monoptah",
                    Some("thread"),
                    Some(metrics),
                )
                .unwrap();
            assert_eq!(
                restored,
                Usage {
                    used: 46_894,
                    window: Some(500_000)
                }
            );
        }
        let final_iteration = Usage {
            used: 2050,
            window: Some(500_000),
        };
        let metrics = crate::agent::TurnMetrics {
            input_tokens: 90_000,
            ..Default::default()
        };
        assert_eq!(
            final_iteration.restore_local_claude(
                &account,
                r"\\?\C:\dev\monoptah",
                Some("thread"),
                Some(metrics)
            ),
            Some(final_iteration)
        );
    }

    #[test]
    fn claude_context_after_compact_reopens_with_the_post_compaction_reading() {
        let root = tempfile::tempdir().unwrap();
        let account = Profile::new("fixture", root.path());
        let project = account
            .home(Kind::Claude)
            .unwrap()
            .join("projects/-project");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("thread.jsonl"), concat!(
            "{\"type\":\"system\",\"subtype\":\"compact_boundary\",\"sessionId\":\"thread\",\"compactMetadata\":{\"trigger\":\"manual\",\"preTokens\":52966,\"postTokens\":12022,\"cumulativeDroppedTokens\":259361}}\n",
            "{\"type\":\"user\",\"sessionId\":\"thread\",\"isCompactSummary\":true}\n"
        )).unwrap();
        let saved = Usage {
            used: 52_966,
            window: Some(272_000),
        };
        assert_eq!(
            saved.restore_local_claude(&account, "/project", Some("thread"), None),
            Some(Usage {
                used: 12_022,
                ..saved
            })
        );
    }

    #[test]
    fn claude_context_after_compact_a_newer_request_supersedes_the_saved_boundary() {
        let root = tempfile::tempdir().unwrap();
        let account = Profile::new("fixture", root.path());
        let project = account
            .home(Kind::Claude)
            .unwrap()
            .join("projects/-project");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("thread.jsonl"), concat!(
            "{\"type\":\"system\",\"subtype\":\"compact_boundary\",\"sessionId\":\"thread\",\"compactMetadata\":{\"postTokens\":12022}}\n",
            "{\"type\":\"assistant\",\"sessionId\":\"thread\",\"message\":{\"usage\":{\"input_tokens\":13000,\"output_tokens\":80}}}\n"
        )).unwrap();
        let saved = Usage {
            used: 12_022,
            window: Some(272_000),
        };
        assert_eq!(
            saved.restore_local_claude(&account, "/project", Some("thread"), None),
            Some(Usage {
                used: 13_080,
                ..saved
            })
        );
        let final_reading = Usage {
            used: 13_100,
            ..saved
        };
        assert_eq!(
            final_reading.restore_local_claude(&account, "/project", Some("thread"), None),
            Some(final_reading)
        );
    }

    #[test]
    fn context_does_not_restore_an_impossible_total_or_cross_a_compaction_boundary() {
        let root = tempfile::tempdir().unwrap();
        let account = Profile::new("fixture", root.path());
        let saved = Usage {
            used: 1_647_660,
            window: Some(500_000),
        };
        assert_eq!(
            saved.restore_local_claude(&account, "/project", Some("../wrong"), None),
            None
        );
        let project = account
            .home(Kind::Claude)
            .unwrap()
            .join("projects/-project");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("thread.jsonl"), b"{\"type\":\"assistant\",\"sessionId\":\"thread\",\"message\":{\"usage\":{\"input_tokens\":100}}}\n{\"type\":\"system\",\"subtype\":\"compact_boundary\",\"sessionId\":\"thread\"}\n").unwrap();
        let valid = Usage { used: 100, ..saved };
        let metrics = crate::agent::TurnMetrics {
            input_tokens: 100,
            ..Default::default()
        };
        assert_eq!(
            valid.restore_local_claude(&account, "/project", Some("thread"), Some(metrics)),
            None
        );
        assert_eq!(
            valid.restore_local_claude(&account, "/project", Some("missing"), None),
            Some(valid)
        );
    }
}
