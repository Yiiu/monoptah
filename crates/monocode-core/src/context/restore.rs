use super::{ClaudeMessageUsage, Usage};
use crate::accounts::Profile;
use crate::cloud_sessions::{claude_project_dir_name, plain_path};
use crate::provider::Kind;
use serde::Deserialize;
use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

impl Usage {
    /// Older clients saved Claude's turn totals as context occupancy. Recover the
    /// latest main request on reopen, including totals that happen to fit the window.
    pub fn restore_local_claude(
        self,
        account: &Profile,
        config_dir: Option<&str>,
        cwd: &str,
        thread: Option<&str>,
        turn_metrics: Option<crate::agent::TurnMetrics>,
    ) -> Option<Self> {
        let Some(window) = self.window.filter(|&w| w > 0) else {
            return Some(self);
        };
        if self.current {
            return Some(self);
        }
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
            let file = transcript_file(account, config_dir, Path::new(cwd), thread)?;
            latest_used(file, thread, keep_saved.then_some(self.used))
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

fn transcript_file(
    account: &Profile,
    config_dir: Option<&str>,
    cwd: &Path,
    thread: &str,
) -> Option<File> {
    crate::accounts::transfer::codex::validate_thread_id(thread).ok()?;
    // Launch applies the account environment after the session overrides,
    // then starts Claude in the project's working directory.
    let home = match (account.directory(Kind::Claude).ok()?, config_dir) {
        (Some(home), _) => home,
        (None, Some("")) => crate::accounts::home()?.join(".claude"),
        (None, Some(dir)) => PathBuf::from(dir),
        (None, None) => account.home(Kind::Claude).ok()?,
    };
    let projects = cwd.join(home).join("projects");
    let cwd = plain_path(cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf()));
    let project = claude_project_dir_name(cwd.to_str()?);
    let name = format!("{thread}.jsonl");
    let preferred = projects.join(project).join(&name);
    if preferred.is_file() {
        return File::open(preferred).ok();
    }
    // Long project names have a CLI-specific hash. A resumed session can
    // also live under its original directory. Never guess between copies.
    let mut matched = None;
    for entry in projects.read_dir().ok()? {
        let entry = entry.ok()?;
        if !entry.file_type().ok()?.is_dir() {
            continue;
        }
        let candidate = entry.path().join(&name);
        if candidate.symlink_metadata().is_ok_and(|m| m.is_file()) {
            if matched.is_some() {
                return None;
            }
            matched = Some(candidate);
        }
    }
    File::open(matched?).ok()
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

fn latest_used(mut file: File, thread: &str, saved: Option<u64>) -> Option<HistoryReading> {
    // A transcript can be hundreds of MB. Inspect only its tail and deserialize
    // the fields we need, without materializing tool results or message bodies.
    const TAIL: u64 = 2 * 1024 * 1024;
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
                let Some(used) = message
                    .usage
                    .and_then(|usage| ClaudeMessageUsage::default().observe(&usage))
                else {
                    continue;
                };
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
    fn context_keeps_a_confirmed_request_equal_to_the_turn_total_after_reopen() {
        use crate::store::Store;
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("config/projects/project-hash");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("thread.jsonl"), r#"{"type":"assistant","sessionId":"thread","message":{"usage":{"input_tokens":150001}}}"#).unwrap();
        let cwd = dir.path().to_str().unwrap();
        let metrics = crate::agent::TurnMetrics {
            input_tokens: 170_000,
            ..Default::default()
        };
        let recover = |usage: Usage| {
            usage
                .restore_local_claude(
                    &Profile::default(),
                    Some("config"),
                    cwd,
                    Some("thread"),
                    Some(metrics),
                )
                .unwrap()
        };
        let mut reading = Usage::default();
        reading.merge(Some(170_000), Some(200_000));
        for (name, open) in [
            ("native", Store::open as fn(&Path) -> Result<Store, String>),
            ("shared", Store::open_monocode),
        ] {
            let path = dir.path().join(name);
            let store = open(&path).unwrap();
            let id = store.create("claude", cwd).unwrap().id;
            store.save_context_usage(id, &reading).unwrap();
            drop(store);
            let store = open(&path).unwrap();
            assert_eq!(recover(store.context_usage(id).unwrap().unwrap()), reading);
            let mut legacy = reading;
            legacy.current = false;
            store.save_context_usage(id, &legacy).unwrap();
            assert_eq!(
                recover(store.context_usage(id).unwrap().unwrap()).used,
                150_001
            );
        }
        // Two same-ID transcripts under unrelated project keys are ambiguous.
        let duplicate = dir.path().join("config/projects/other-project");
        fs::create_dir_all(&duplicate).unwrap();
        fs::copy(project.join("thread.jsonl"), duplicate.join("thread.jsonl")).unwrap();
        assert_eq!(
            recover(Usage {
                current: false,
                ..reading
            })
            .used,
            170_000
        );
    }

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
                ..Default::default()
            };
            let metrics = crate::agent::TurnMetrics {
                input_tokens: old,
                ..Default::default()
            };
            let restored = saved
                .restore_local_claude(
                    &account,
                    None,
                    r"\\?\C:\dev\monoptah",
                    Some("thread"),
                    Some(metrics),
                )
                .unwrap();
            assert_eq!(
                restored,
                Usage {
                    used: 46_894,
                    window: Some(500_000),
                    ..Default::default()
                }
            );
        }
        let final_iteration = Usage {
            used: 2050,
            window: Some(500_000),
            ..Default::default()
        };
        let metrics = crate::agent::TurnMetrics {
            input_tokens: 90_000,
            ..Default::default()
        };
        assert_eq!(
            final_iteration.restore_local_claude(
                &account,
                None,
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
            "{\"type\":\"assistant\",\"sessionId\":\"thread\",\"message\":{\"content\":[]}}\n",
            "{\"type\":\"assistant\",\"sessionId\":\"thread\",\"message\":{\"usage\":{\"input_tokens\":0}}}\n",
            "{\"type\":\"user\",\"sessionId\":\"thread\",\"isCompactSummary\":true}\n"
        )).unwrap();
        let saved = Usage {
            used: 52_966,
            window: Some(272_000),
            ..Default::default()
        };
        assert_eq!(
            saved.restore_local_claude(&account, None, "/project", Some("thread"), None),
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
            ..Default::default()
        };
        assert_eq!(
            saved.restore_local_claude(&account, None, "/project", Some("thread"), None),
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
            final_reading.restore_local_claude(&account, None, "/project", Some("thread"), None),
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
            ..Default::default()
        };
        assert_eq!(
            saved.restore_local_claude(&account, None, "/project", Some("../wrong"), None),
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
            valid.restore_local_claude(&account, None, "/project", Some("thread"), Some(metrics)),
            None
        );
        assert_eq!(
            valid.restore_local_claude(&account, None, "/project", Some("missing"), None),
            Some(valid)
        );
    }
}
