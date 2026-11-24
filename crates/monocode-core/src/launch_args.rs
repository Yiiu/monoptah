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
/// stays in `rest`. Nothing at or after the end-of-options marker `--` is taken.
pub fn extract_structured(kind: Kind, args: &[String]) -> Extracted {
    let mut out = Extracted::default();
    // Codex's `-a` and `-s` are decided together once every flag is seen. In typed order:
    // where in `rest` the flag goes back if it stays, whether it is the sandbox, its tokens.
    let mut pending: Vec<(usize, bool, Vec<String>)> = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--" {
            out.rest.extend(args[index..].iter().cloned());
            break;
        }
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        // The flag's value: inline (`--flag=value`) or the next argument.
        let takes = |index: usize| -> Option<(String, usize)> {
            match &inline {
                Some(value) => Some((value.clone(), 1)),
                // The end-of-options marker is never a value: the flag stays, unmoved.
                None => args
                    .get(index + 1)
                    .filter(|value| *value != "--")
                    .map(|value| (value.clone(), 2)),
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
            (Kind::Codex, "-s" | "--sandbox" | "-a" | "--ask-for-approval") => {
                // A permission level only together with the other flag; decided below.
                if let Some((_, used)) = takes(index) {
                    pending.push((
                        out.rest.len(),
                        matches!(flag, "-s" | "--sandbox"),
                        args[index..index + used].to_vec(),
                    ));
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
    // Only a pair that a level launches with exactly moves (`tui_launch::codex_flags`):
    // any other pair, or either flag alone, would change the approval policy or the
    // sandbox Codex runs with, so both stay where they were typed.
    let value = |sandbox: bool| -> Option<String> {
        let (_, _, tokens) = pending.iter().rev().find(|(_, is, _)| *is == sandbox)?;
        Some(match tokens.as_slice() {
            [_, value] => value.clone(),
            [inline] => inline
                .split_once('=')
                .map(|(_, value)| value.to_owned())
                .unwrap_or_default(),
            _ => String::new(),
        })
    };
    let level = match (value(false), value(true)) {
        (Some(approval), Some(sandbox)) => match (approval.as_str(), sandbox.as_str()) {
            ("on-request", "read-only") => Some(AccessMode::Supervised),
            ("on-request", "workspace-write") => Some(AccessMode::AutoAcceptEdits),
            ("never", "danger-full-access") => Some(AccessMode::FullAccess),
            _ => None,
        },
        _ => None,
    };
    if let Some(level) = level {
        out.permission = Some(level);
        out.moved
            .extend(pending.into_iter().map(|(_, _, tokens)| tokens.join(" ")));
    } else {
        // Back where they were typed, last first so earlier positions stay valid.
        for (at, _, tokens) in pending.into_iter().rev() {
            out.rest.splice(at..at, tokens);
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
            if let Some(effort) = effort.and_then(|e| claude_chat_effort(e, model.as_deref())) {
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

/// The `--effort` value a Claude chat launch passes for `effort` with native `model`, or
/// none. `ultrathink` is a prompt keyword, not a flag; `ultracode` runs at `xhigh` (the
/// launch also turns on its setting); models older than `xhigh` take `max` instead, and
/// Sonnet 4.6, which has no `max`, takes `high`.
pub fn claude_chat_effort<'a>(effort: &'a str, model: Option<&str>) -> Option<&'a str> {
    const BEFORE_XHIGH: [&str; 8] = [
        "claude-opus-4-6",
        "claude-sonnet-4-6",
        "claude-opus-4-5",
        "claude-haiku-4-5",
        "claude-opus-4-1",
        "claude-opus-4-0",
        "claude-sonnet-4-0",
        "claude-sonnet-4-5",
    ];
    // In order, each step on the previous one's result: `ultracode` becomes `xhigh`, an
    // older model's `xhigh` becomes `max`, and Sonnet 4.6's `max` becomes `high`.
    let mut effort = match effort {
        "ultrathink" => return None,
        "ultracode" => "xhigh",
        other => other,
    };
    if effort == "xhigh" && model.is_some_and(|m| BEFORE_XHIGH.contains(&m)) {
        effort = "max";
    }
    if effort == "max" && model == Some("claude-sonnet-4-6") {
        effort = "high";
    }
    Some(effort)
}

/// Whether Settings keeps model, effort and permission flags typed into `kind`'s arguments
/// where they are, instead of moving them into its fields, for agents that open in a
/// terminal. Such a terminal passes no flags of its own (`tui_caps`), so a moved flag
/// would never reach it.
pub fn terminal_keeps_typed_flags(kind: Kind) -> bool {
    !crate::surface::tui_caps(kind).launch_flags
}

/// The user's arguments as a chat launch passes them. Where a terminal kept typed model,
/// effort and permission flags (`terminal_keeps_typed_flags`), a chat drops them: the chat
/// takes those from Settings and places them itself, and a subcommand's flag in front of
/// the subcommand (Grok's `--model` before `agent`) would not parse.
pub fn chat_user_args(kind: Kind, args: &[String]) -> Vec<String> {
    if terminal_keeps_typed_flags(kind) {
        extract_structured(kind, args).rest
    } else {
        args.to_vec()
    }
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
    fn codex_moves_only_an_approval_and_sandbox_pair_a_level_launches_with() {
        let pairs: [(&[&str], AccessMode); 3] = [
            (
                &["-a", "on-request", "-s", "read-only"],
                AccessMode::Supervised,
            ),
            (
                &[
                    "--sandbox=workspace-write",
                    "--ask-for-approval",
                    "on-request",
                ],
                AccessMode::AutoAcceptEdits,
            ),
            (
                &["-s", "danger-full-access", "-a", "never"],
                AccessMode::FullAccess,
            ),
        ];
        for (pair, level) in pairs {
            let out = extract_structured(Kind::Codex, &strings(pair));
            assert_eq!(out.permission, Some(level), "{pair:?}");
            assert!(out.rest.is_empty(), "{pair:?}");
        }
        // `never` with a writable sandbox is no level: Auto-accept edits asks on request.
        // Both stay exactly where they were typed.
        let typed = strings(&[
            "--search",
            "-a",
            "never",
            "--verbose",
            "-s",
            "workspace-write",
        ]);
        let out = extract_structured(Kind::Codex, &typed);
        assert_eq!(out.permission, None);
        assert!(out.moved.is_empty());
        assert_eq!(out.rest, typed);
        // A sandbox alone keeps the configured approval policy, so it stays too.
        let alone = extract_structured(Kind::Codex, &strings(&["-s", "read-only"]));
        assert_eq!(alone.permission, None);
        assert_eq!(alone.rest, strings(&["-s", "read-only"]));
    }

    #[test]
    fn a_chat_drops_the_flags_a_terminal_kept_and_claude_keeps_its_arguments() {
        assert!(terminal_keeps_typed_flags(Kind::Grok));
        assert!(!terminal_keeps_typed_flags(Kind::Claude));
        let typed = strings(&[
            "--verbose",
            "--model",
            "grok-4",
            "--reasoning-effort",
            "high",
        ]);
        assert_eq!(chat_user_args(Kind::Grok, &typed), strings(&["--verbose"]));
        let claude = strings(&["--add-dir", "/work"]);
        assert_eq!(chat_user_args(Kind::Claude, &claude), claude);
    }

    #[test]
    fn nothing_after_the_end_of_options_marker_is_moved() {
        let marker_as_value = strings(&["--model", "--", "--effort", "high"]);
        let out = extract_structured(Kind::Claude, &marker_as_value);
        assert_eq!((out.model, out.effort), (None, None));
        assert_eq!(out.rest, marker_as_value);
        let out = extract_structured(
            Kind::Codex,
            &strings(&[
                "-m",
                "gpt-5",
                "--",
                "--model",
                "literal",
                "-a",
                "on-request",
            ]),
        );
        assert_eq!(out.model.as_deref(), Some("gpt-5"));
        assert_eq!(out.moved, strings(&["-m gpt-5"]));
        assert_eq!(
            out.rest,
            strings(&["--", "--model", "literal", "-a", "on-request"])
        );
        let pair = extract_structured(
            Kind::Codex,
            &strings(&["-a", "on-request", "--", "-s", "read-only"]),
        );
        assert_eq!(pair.permission, None);
        assert_eq!(
            pair.rest,
            strings(&["-a", "on-request", "--", "-s", "read-only"])
        );
    }

    #[test]
    fn the_claude_chat_preview_passes_the_effort_the_launch_does() {
        let effort = |model: &str, effort: &str| {
            let args = structured_flags(
                Kind::Claude,
                false,
                Some(model),
                Some(effort),
                AccessMode::AgentDefault,
            );
            args.iter()
                .position(|a| a == "--effort")
                .map(|at| args[at + 1].clone())
        };
        assert_eq!(effort("claude-sonnet-4-6", "max").as_deref(), Some("high"));
        assert_eq!(effort("claude-opus-4-5", "xhigh").as_deref(), Some("max"));
        assert_eq!(effort("claude-sonnet-4-6", "high").as_deref(), Some("high"));
        assert_eq!(effort("claude-opus-4-5", "ultrathink"), None);
        assert_eq!(claude_chat_effort("ultracode", None), Some("xhigh"));
        // Each step applies to the previous one's result.
        assert_eq!(
            effort("claude-sonnet-4-6", "xhigh").as_deref(),
            Some("high")
        );
        assert_eq!(
            effort("claude-sonnet-4-6", "ultracode").as_deref(),
            Some("high")
        );
        assert_eq!(
            claude_chat_effort("max", Some("claude-opus-4-6")),
            Some("max")
        );
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
