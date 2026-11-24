//! Per-agent launch arguments and environment from Settings → Agents: parsing the typed
//! text, moving known flags into their structured fields, and adding both to a launch.
//!
//! Environment values are stored in plain text in the workspace settings. That is a
//! deliberate exception to the persistence rule against plain-text tokens (design of the
//! agent settings redesign, 2026-11-24); views mask values whose name `is_secret`.
use crate::agent::AccessMode;
use crate::provider::Kind;
use serde::Serialize;
use std::process::Command;

/// What one agent's launches add: arguments (already split) and `KEY=VALUE` variables.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Launch {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl Launch {
    pub fn is_empty(&self) -> bool {
        self.args.is_empty() && self.env.is_empty()
    }
}

/// Split an argument line with shell rules. An unclosed quote or a trailing escape is an
/// error, so a line that cannot be split is never saved.
pub fn split(line: &str) -> Result<Vec<String>, String> {
    if line.trim().is_empty() {
        return Ok(Vec::new());
    }
    shlex::split(line).ok_or_else(|| {
        "These arguments could not be split. Check for an unclosed quote or a trailing backslash."
            .into()
    })
}

/// Parse `KEY=VALUE` lines. Blank lines and lines starting with `#` are skipped; a later
/// line for the same name replaces the earlier one.
pub fn parse_env(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("Line {} is not KEY=VALUE.", index + 1));
        };
        let key = key.trim();
        if !valid_key(key) {
            return Err(format!(
                "Line {}: \"{key}\" is not a valid variable name.",
                index + 1
            ));
        }
        out.retain(|(existing, _)| existing != key);
        out.push((key.to_owned(), value.to_owned()));
    }
    Ok(out)
}

fn valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether a variable's value is masked until revealed.
pub fn is_secret(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "AUTH"]
        .iter()
        .any(|part| key.contains(part))
}

/// The flags `extract_structured` took out of an argument list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extracted {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission: Option<AccessMode>,
    /// The moved flags as typed (with their values), for the notice.
    pub moved: Vec<String>,
    /// Every other argument, unchanged and in order.
    pub rest: Vec<String>,
}

/// Take the model, effort and permission flags this agent is known to accept out of `args`.
/// Anything else, including a known flag with a value that maps to no permission level,
/// stays in `rest`.
pub fn extract_structured(kind: Kind, args: &[String]) -> Extracted {
    let mut out = Extracted::default();
    let mut sandbox_moved = false;
    let mut approval: Option<Vec<String>> = None;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        // The flag's value: inline (`--flag=value`) or the next argument.
        let takes = |index: usize| -> Option<(String, usize)> {
            match &inline {
                Some(value) => Some((value.clone(), 1)),
                None => args.get(index + 1).map(|value| (value.clone(), 2)),
            }
        };
        let mut taken: Option<usize> = None;
        match (kind, flag) {
            (_, "--model") | (Kind::Codex, "-m") => {
                if let Some((value, used)) = takes(index) {
                    out.model = Some(value);
                    taken = Some(used);
                }
            }
            (Kind::Claude, "--effort") | (Kind::Grok, "--reasoning-effort") => {
                if let Some((value, used)) = takes(index) {
                    out.effort = Some(value);
                    taken = Some(used);
                }
            }
            (Kind::Codex, "-c" | "--config") => {
                if let Some((value, used)) = takes(index) {
                    if let Some((key, setting)) = value.split_once('=') {
                        let setting = unquote(setting);
                        match key.trim() {
                            "model" => {
                                out.model = Some(setting);
                                taken = Some(used);
                            }
                            "model_reasoning_effort" => {
                                out.effort = Some(setting);
                                taken = Some(used);
                            }
                            _ => {}
                        }
                    }
                }
            }
            (Kind::Claude, "--permission-mode") => {
                if let Some((value, used)) = takes(index) {
                    let level = match value.as_str() {
                        "default" | "manual" => Some(AccessMode::Supervised),
                        "acceptEdits" => Some(AccessMode::AutoAcceptEdits),
                        "auto" => Some(AccessMode::Auto),
                        "bypassPermissions" => Some(AccessMode::FullAccess),
                        _ => None,
                    };
                    if let Some(level) = level {
                        out.permission = Some(level);
                        taken = Some(used);
                    }
                }
            }
            (Kind::Claude, "--dangerously-skip-permissions")
            | (Kind::Codex, "--dangerously-bypass-approvals-and-sandbox" | "--yolo")
            | (Kind::Grok, "--always-approve") => {
                out.permission = Some(AccessMode::FullAccess);
                taken = Some(1);
            }
            (Kind::Codex, "--approve-for-me") => {
                out.permission = Some(AccessMode::Auto);
                taken = Some(1);
            }
            (Kind::Codex, "-s" | "--sandbox") => {
                if let Some((value, used)) = takes(index) {
                    let level = match value.as_str() {
                        "read-only" => Some(AccessMode::Supervised),
                        "workspace-write" => Some(AccessMode::AutoAcceptEdits),
                        "danger-full-access" => Some(AccessMode::FullAccess),
                        _ => None,
                    };
                    if let Some(level) = level {
                        out.permission = Some(level);
                        sandbox_moved = true;
                        taken = Some(used);
                    }
                }
            }
            (Kind::Codex, "-a" | "--ask-for-approval") => {
                // Only part of a permission level together with a sandbox; decided below.
                if let Some((_, used)) = takes(index) {
                    approval = Some(args[index..index + used].to_vec());
                    index += used;
                    continue;
                }
            }
            _ => {}
        }
        match taken {
            Some(used) => {
                out.moved.push(args[index..index + used].join(" "));
                index += used;
            }
            None => {
                out.rest.push(arg.clone());
                index += 1;
            }
        }
    }
    if let Some(approval) = approval {
        if sandbox_moved {
            out.moved.push(approval.join(" "));
        } else {
            out.rest.extend(approval);
        }
    }
    out
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(value)
        .to_owned()
}

/// Whether the agent's chat launch starts with a transport subcommand (`codex app-server`,
/// `<cli> acp`, Grok's `agent … stdio`, `opencode serve`): user arguments go in front of it
/// so they act as global options. Claude, Pi and Omp take them after their own flags.
pub fn args_lead(kind: Kind) -> bool {
    !matches!(kind, Kind::Claude | Kind::Pi | Kind::Omp)
}

/// The reasoning effort levels Settings → Agents offers for an agent, lowest first. Empty
/// where the agent takes no effort setting of its own (its models advertise their options
/// per session instead): the field is hidden.
pub fn effort_levels(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Claude => &["low", "medium", "high", "xhigh", "max"],
        Kind::Codex | Kind::Pi | Kind::Omp => &["minimal", "low", "medium", "high", "xhigh"],
        Kind::Grok | Kind::Hermes => &["low", "medium", "high"],
        _ => &[],
    }
}

/// The model, effort and permission flags a new session of `kind` puts on its command
/// line. A terminal session passes `tui_launch::structured_flags`; a chat session passes
/// flags only where its transport takes them on the command line (Claude, Grok, Fx's
/// model). The other chat transports send these settings over their protocol, so nothing
/// is listed for them.
pub fn structured_flags(
    kind: Kind,
    terminal: bool,
    model: Option<&str>,
    effort: Option<&str>,
    access: AccessMode,
) -> Vec<String> {
    if terminal {
        return crate::tui_launch::structured_flags(kind, model, effort, access);
    }
    let model = model.and_then(|m| crate::store::native_model(m, kind.id()));
    let effort = effort.filter(|e| !e.trim().is_empty());
    let mut args: Vec<String> = Vec::new();
    match kind {
        Kind::Claude => {
            if let Some(mode) = access.claude() {
                args.extend(["--permission-mode".into(), mode.into()]);
                if mode == "bypassPermissions" {
                    args.push("--allow-dangerously-skip-permissions".into());
                }
            }
            if let Some(effort) = effort {
                args.extend(["--effort".into(), effort.into()]);
            }
            if let Some(model) = model {
                args.extend(["--model".into(), model]);
            }
        }
        Kind::Grok => {
            if let Some(model) = model {
                args.extend(["--model".into(), model]);
            }
            if let Some(effort) = effort {
                args.extend(["--reasoning-effort".into(), effort.into()]);
            }
            if access == AccessMode::FullAccess {
                args.push("--always-approve".into());
            }
        }
        Kind::Fx => {
            if let Some(model) = model {
                args.extend(["--model".into(), model]);
            }
        }
        _ => {}
    }
    args
}

/// Whether a chat session of `kind` sends its model, effort and permission over the
/// agent's protocol instead of the command line, so `structured_flags` lists none of them.
pub fn chat_uses_protocol(kind: Kind) -> bool {
    !matches!(kind, Kind::Claude | Kind::Grok)
}

/// Arguments joined back into one line that `split` reads the same way.
pub fn join(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            shlex::try_quote(arg)
                .map(|quoted| quoted.into_owned())
                .unwrap_or_else(|_| arg.clone())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Add the user's variables. Call after the app's own session environment and before the
/// account (`Profile::apply`), so the account wins a collision.
pub fn apply_env(command: &mut Command, env: &[(String, String)]) {
    for (key, value) in env {
        command.env(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn splits_with_shell_rules_and_rejects_unclosed_quotes() {
        assert_eq!(
            split(r#"--add-dir "a b" --flag='x y' plain"#).unwrap(),
            strings(&["--add-dir", "a b", "--flag=x y", "plain"])
        );
        assert_eq!(split("   ").unwrap(), Vec::<String>::new());
        assert!(split(r#"--name "unclosed"#).is_err());
        assert!(split("--name 'unclosed").is_err());
    }

    #[test]
    fn env_lines_parse_skip_comments_and_reject_bad_lines() {
        let env = parse_env(
            "# proxy\nHTTP_PROXY=http://127.0.0.1:8080\n\nA=1\nA=2\nEMPTY=\nB= keep spaces ",
        )
        .unwrap();
        assert_eq!(
            env,
            vec![
                ("HTTP_PROXY".into(), "http://127.0.0.1:8080".into()),
                ("A".into(), "2".into()),
                ("EMPTY".into(), String::new()),
                ("B".into(), " keep spaces".into()),
            ]
        );
        assert_eq!(
            parse_env("A=1\nnot a pair").unwrap_err(),
            "Line 2 is not KEY=VALUE."
        );
        assert!(parse_env("1BAD=x").is_err());
        assert!(parse_env("BAD-NAME=x").is_err());
        assert!(parse_env("=x").is_err());
    }

    #[test]
    fn joined_arguments_split_back_to_the_same_list() {
        let args = strings(&["--add-dir", "a b", "--flag=x y", "plain", "it's"]);
        assert_eq!(split(&join(&args)).unwrap(), args);
        assert_eq!(join(&strings(&["--search"])), "--search");
    }

    #[test]
    fn the_preview_flags_follow_the_session_kind() {
        // A terminal session passes exactly what `tui_launch::build` does.
        assert_eq!(
            structured_flags(
                Kind::Claude,
                true,
                Some("opus"),
                Some("high"),
                AccessMode::AutoAcceptEdits
            ),
            strings(&[
                "--model",
                "opus",
                "--permission-mode",
                "acceptEdits",
                "--effort",
                "high"
            ])
        );
        assert_eq!(
            structured_flags(Kind::Codex, true, None, None, AccessMode::FullAccess),
            strings(&["--dangerously-bypass-approvals-and-sandbox"])
        );
        // The agent's own settings pass no permission flag anywhere.
        for kind in Kind::ALL {
            for terminal in [true, false] {
                assert!(
                    structured_flags(kind, terminal, None, None, AccessMode::AgentDefault)
                        .is_empty(),
                    "{kind:?} {terminal}"
                );
            }
        }
        // Chat: Claude takes flags, Codex sends everything over its protocol.
        assert_eq!(
            structured_flags(Kind::Claude, false, None, None, AccessMode::FullAccess),
            strings(&[
                "--permission-mode",
                "bypassPermissions",
                "--allow-dangerously-skip-permissions"
            ])
        );
        assert!(structured_flags(
            Kind::Codex,
            false,
            Some("gpt-5"),
            Some("high"),
            AccessMode::Auto
        )
        .is_empty());
        assert!(chat_uses_protocol(Kind::Codex));
        assert!(!chat_uses_protocol(Kind::Claude));
        // A terminal of an agent whose flags are not verified gets none.
        assert!(structured_flags(
            Kind::Grok,
            true,
            Some("grok-4"),
            None,
            AccessMode::FullAccess
        )
        .is_empty());
    }

    #[test]
    fn effort_levels_exist_only_where_the_agent_takes_one() {
        assert_eq!(effort_levels(Kind::Claude).last(), Some(&"max"));
        assert!(effort_levels(Kind::Codex).contains(&"minimal"));
        assert!(effort_levels(Kind::Cursor).is_empty());
    }

    #[test]
    fn secret_names_are_recognised_case_insensitively() {
        for key in [
            "OPENAI_API_KEY",
            "github_token",
            "CLIENT_SECRET",
            "DB_PASSWORD",
            "AUTH_HEADER",
        ] {
            assert!(is_secret(key), "{key}");
        }
        for key in ["HTTP_PROXY", "RUST_LOG", "PATH"] {
            assert!(!is_secret(key), "{key}");
        }
    }

    #[test]
    fn claude_flags_move_into_their_fields() {
        let out = extract_structured(
            Kind::Claude,
            &strings(&[
                "--add-dir",
                "/work",
                "--model=opus",
                "--permission-mode",
                "acceptEdits",
                "--effort",
                "high",
                "--verbose",
            ]),
        );
        assert_eq!(out.model.as_deref(), Some("opus"));
        assert_eq!(out.effort.as_deref(), Some("high"));
        assert_eq!(out.permission, Some(AccessMode::AutoAcceptEdits));
        assert_eq!(
            out.moved,
            strings(&[
                "--model=opus",
                "--permission-mode acceptEdits",
                "--effort high"
            ])
        );
        assert_eq!(out.rest, strings(&["--add-dir", "/work", "--verbose"]));
    }

    #[test]
    fn a_permission_value_with_no_level_stays_in_the_arguments() {
        let out = extract_structured(Kind::Claude, &strings(&["--permission-mode", "plan"]));
        assert_eq!(out.permission, None);
        assert_eq!(out.rest, strings(&["--permission-mode", "plan"]));
        let full = extract_structured(Kind::Claude, &strings(&["--dangerously-skip-permissions"]));
        assert_eq!(full.permission, Some(AccessMode::FullAccess));
        assert!(full.rest.is_empty());
    }

    #[test]
    fn codex_flags_move_including_config_overrides() {
        let out = extract_structured(
            Kind::Codex,
            &strings(&[
                "-m",
                "gpt-5",
                "-c",
                "model_reasoning_effort=\"high\"",
                "-a",
                "on-request",
                "-s",
                "workspace-write",
                "-c",
                "tools.web_search=true",
            ]),
        );
        assert_eq!(out.model.as_deref(), Some("gpt-5"));
        assert_eq!(out.effort.as_deref(), Some("high"));
        assert_eq!(out.permission, Some(AccessMode::AutoAcceptEdits));
        assert_eq!(out.rest, strings(&["-c", "tools.web_search=true"]));
        assert!(out.moved.contains(&"-a on-request".to_string()));
        // An approval policy alone is no permission level: it stays.
        let alone = extract_structured(Kind::Codex, &strings(&["-a", "never"]));
        assert_eq!(alone.permission, None);
        assert_eq!(alone.rest, strings(&["-a", "never"]));
        let yolo = extract_structured(Kind::Codex, &strings(&["--yolo"]));
        assert_eq!(yolo.permission, Some(AccessMode::FullAccess));
        let auto = extract_structured(Kind::Codex, &strings(&["--approve-for-me"]));
        assert_eq!(auto.permission, Some(AccessMode::Auto));
    }

    #[test]
    fn other_agents_only_move_the_model() {
        let out = extract_structured(
            Kind::Hermes,
            &strings(&["--model", "m", "--effort", "high", "--yolo"]),
        );
        assert_eq!(out.model.as_deref(), Some("m"));
        assert_eq!(out.effort, None);
        assert_eq!(out.permission, None);
        assert_eq!(out.rest, strings(&["--effort", "high", "--yolo"]));
        let grok = extract_structured(
            Kind::Grok,
            &strings(&["--reasoning-effort=low", "--always-approve"]),
        );
        assert_eq!(grok.effort.as_deref(), Some("low"));
        assert_eq!(grok.permission, Some(AccessMode::FullAccess));
    }

    #[test]
    fn a_trailing_flag_without_its_value_is_left_alone() {
        let out = extract_structured(Kind::Claude, &strings(&["--model"]));
        assert_eq!(out.model, None);
        assert_eq!(out.rest, strings(&["--model"]));
    }

    #[test]
    fn transport_agents_take_arguments_in_front() {
        for kind in Kind::ALL {
            assert_eq!(
                args_lead(kind),
                !matches!(kind, Kind::Claude | Kind::Pi | Kind::Omp),
                "{kind:?}"
            );
        }
    }
}
