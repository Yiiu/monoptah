//! Composer settings and approval decisions shared by the native view and transports.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccessMode {
    /// Pass no permission flag or parameters: the CLI applies its own configuration.
    #[serde(rename = "own")]
    AgentDefault,
    ReadOnly,
    #[default]
    Supervised,
    AutoAcceptEdits,
    Auto,
    FullAccess,
}
impl AccessMode {
    pub fn id(self) -> &'static str {
        match self {
            Self::AgentDefault => "own",
            Self::ReadOnly => "read-only",
            Self::Supervised => "supervised",
            Self::AutoAcceptEdits => "auto-accept-edits",
            Self::Auto => "auto",
            Self::FullAccess => "full-access",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::AgentDefault => "Agent's own settings",
            Self::ReadOnly => "Read-only",
            Self::Supervised => "Supervised",
            Self::AutoAcceptEdits => "Auto-accept edits",
            Self::Auto => "Auto",
            Self::FullAccess => "Full access",
        }
    }
    pub fn from_id(id: &str) -> Self {
        match id {
            "own" => Self::AgentDefault,
            "read-only" => Self::ReadOnly,
            "auto-accept-edits" => Self::AutoAcceptEdits,
            "auto" => Self::Auto,
            "full-access" => Self::FullAccess,
            _ => Self::Supervised,
        }
    }
    /// The id a peer that predates `AgentDefault` understands (the remote Host and phone
    /// clients): the agent's own settings read as supervised there.
    pub fn peer_id(self) -> &'static str {
        match self {
            Self::AgentDefault => Self::Supervised.id(),
            other => other.id(),
        }
    }
    /// codexProtocol.ts::runtimeModeToCodexConfig. `None` sends no approval or sandbox
    /// parameters, so Codex applies its own configuration.
    ///
    /// Checked against codex-cli 0.172.0: the app-server schema still accepts `untrusted`
    /// (`AskForApproval` is untrusted | on-request | never) although the TUI's `-a` now
    /// lists only on-request and never.
    pub fn codex(self) -> Option<(Value, &'static str, Value, &'static str)> {
        Some(match self {
            Self::AgentDefault => return None,
            Self::ReadOnly => (
                json!("never"),
                "read-only",
                json!({"type":"readOnly"}),
                "user",
            ),
            Self::Supervised => (
                json!("untrusted"),
                "read-only",
                json!({"type":"readOnly"}),
                "user",
            ),
            Self::AutoAcceptEdits => (
                json!("on-request"),
                "workspace-write",
                json!({"type":"workspaceWrite"}),
                "user",
            ),
            Self::Auto => (
                json!("on-request"),
                "workspace-write",
                json!({"type":"workspaceWrite"}),
                "auto_review",
            ),
            Self::FullAccess => (
                json!("on-request"),
                "danger-full-access",
                json!({"type":"dangerFullAccess"}),
                "user",
            ),
        })
    }
    /// Claude's `--permission-mode`; `None` passes no flag. claude 2.1.331 lists `manual`
    /// in its help but still accepts `default`, which older releases need, so it stays.
    pub fn claude(self) -> Option<&'static str> {
        Some(match self {
            Self::AgentDefault => return None,
            Self::ReadOnly => "plan",
            Self::Supervised => "default",
            Self::AutoAcceptEdits => "acceptEdits",
            Self::Auto => "auto",
            Self::FullAccess => "bypassPermissions",
        })
    }
}
/// A permission level from Settings → Agents. Sessions carry it as an `AccessMode`;
/// `ReadOnly` stays a composer/plan-only mode and is not a settings level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Permission {
    /// The agent's own settings: no permission flag or parameters.
    Own,
    Ask,
    AcceptEdits,
    /// "Approve for me".
    Auto,
    Full,
}
impl Permission {
    pub const ALL: [Self; 5] = [
        Self::Own,
        Self::Ask,
        Self::AcceptEdits,
        Self::Auto,
        Self::Full,
    ];
    /// The stored id.
    pub fn id(self) -> &'static str {
        match self {
            Self::Own => "own",
            Self::Ask => "ask",
            Self::AcceptEdits => "edits",
            Self::Auto => "auto",
            Self::Full => "full",
        }
    }
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|level| level.id() == id)
    }
    /// The name Settings → Agents shows.
    pub fn label(self) -> &'static str {
        match self {
            Self::Own => "Agent's own settings",
            Self::Ask => "Ask for approval",
            Self::AcceptEdits => "Accept edits",
            Self::Auto => "Approve for me",
            Self::Full => "Full access",
        }
    }
    /// One line on what the level lets an agent do.
    pub fn description(self) -> &'static str {
        match self {
            Self::Own => {
                "Monoptah passes no permission setting. The agent uses its own configuration."
            }
            Self::Ask => "Asks before editing files or running commands.",
            Self::AcceptEdits => "Edits files without asking. Asks before running commands.",
            Self::Auto => {
                "The agent reviews approval requests itself and asks only for risky ones."
            }
            Self::Full => "Never asks. Can edit any file and run any command.",
        }
    }
    pub fn access(self) -> AccessMode {
        match self {
            Self::Own => AccessMode::AgentDefault,
            Self::Ask => AccessMode::Supervised,
            Self::AcceptEdits => AccessMode::AutoAcceptEdits,
            Self::Auto => AccessMode::Auto,
            Self::Full => AccessMode::FullAccess,
        }
    }
    /// The level a session's access mode reads as; read-only reads as ask.
    pub fn from_access(access: AccessMode) -> Self {
        match access {
            AccessMode::AgentDefault => Self::Own,
            AccessMode::ReadOnly | AccessMode::Supervised => Self::Ask,
            AccessMode::AutoAcceptEdits => Self::AcceptEdits,
            AccessMode::Auto => Self::Auto,
            AccessMode::FullAccess => Self::Full,
        }
    }
    /// The levels an agent can apply. Pi and Omp have no permission mechanism (empty: the
    /// field is hidden); Cursor and Fx only approve on the client side.
    pub fn supported(kind: crate::provider::Kind) -> &'static [Self] {
        use crate::provider::Kind;
        match kind {
            Kind::Pi | Kind::Omp => &[],
            Kind::Cursor | Kind::Fx => &[Self::Own, Self::Auto, Self::Full],
            _ => &Self::ALL,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Native remote control invocation, never saved as provider configuration.
    #[serde(skip)]
    pub remote_action: Option<serde_json::Value>,
    pub account: crate::accounts::Profile,
    pub usage_limit: Option<crate::usage::Limit>,
    /// The user turned automatic resume off for this session while the global
    /// "auto resume at usage limit reset" preference was on. The global switch
    /// never arms an opted-out session again; arming it by hand clears this.
    pub auto_resume_opt_out: bool,
    /// Isolated background text jobs; never persisted as conversation settings.
    #[serde(skip)]
    pub text_only: bool,
    /// Transport-only context (for example selected MCP servers). The raw user
    /// message remains the transcript and queue label.
    #[serde(skip)]
    pub prepared_prompt: Option<String>,
    /// Immutable identity captured for this newly submitted turn.
    #[serde(skip)]
    pub turn_model: Option<TurnInfo>,
    #[serde(skip)]
    pub disable_claude_hooks: bool,
    /// Active goal at submission, used to reconcile Claude's delayed result attachment.
    #[serde(skip)]
    pub claude_goal_condition: Option<String>,
    #[serde(skip)]
    pub environment: std::collections::HashMap<String, String>,
    #[serde(skip)]
    pub mono: Option<MonoContext>,
    #[serde(skip)]
    pub mono_message_id: Option<String>,
    #[serde(skip)]
    pub mono_sent_at: Option<u64>,
    #[serde(skip)]
    pub internal_message: bool,
    #[serde(skip)]
    pub attachments: Vec<std::path::PathBuf>,
    pub revision: u64,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub access: AccessMode,
    pub fast: bool,
    pub service_tier: Option<String>,
    pub thinking: Option<bool>,
    /// Provider-advertised controls beyond the common effort/speed switches.
    pub model_settings: std::collections::HashMap<String, String>,
    pub plan: bool,
    pub operator: bool,
    /// A submitted /operator grants this thread access independently of its next draft.
    pub operator_enabled: bool,
    pub orchestrator: bool,
    pub new_worktree: bool,
    pub worktree_base: Option<String>,
    pub worktree_cwd: Option<String>,
    pub worktree_removed: bool,
    pub handoff: Option<Handoff>,
    pub pending_provider_switch: Option<ProviderSwitch>,
    pub turns: std::collections::HashMap<usize, TurnInfo>,
}

impl Settings {
    /// The folder a session with this configuration runs in: its worktree unless that is gone,
    /// else `cwd` (the session's own folder).
    pub fn execution_cwd<'a>(&'a self, cwd: &'a str) -> &'a str {
        if self.worktree_removed {
            return cwd;
        }
        self.worktree_cwd.as_deref().unwrap_or(cwd)
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderSwitch {
    pub from: String,
    /// The outgoing model's display name, for the transfer status.
    pub model_label: String,
    pub cwd: String,
    pub model: Option<String>,
    pub model_settings: std::collections::HashMap<String, String>,
    pub effort: Option<String>,
    pub fast: bool,
    pub service_tier: Option<String>,
    pub thinking: Option<bool>,
    pub skip_outgoing_recap: bool,
    pub account: crate::accounts::Profile,
    pub provider_id: Option<String>,
}
#[derive(Clone, Debug)]
pub struct MonoContext {
    pub mono: crate::mono::Mono,
    pub root: std::path::PathBuf,
    pub executable: std::path::PathBuf,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TurnInfo {
    /// An accepted steer belongs to the provider run that opened this receipt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub joined_turn: Option<usize>,
    pub provider_turn_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub model_id: Option<String>,
    pub duration_ms: u64,
    /// Distinguishes a measured zero from a legacy turn without timing.
    pub duration_known: bool,
    pub started_at: Option<u64>,
    pub completed_at: Option<u64>,
    pub metrics: Option<TurnMetrics>,
    /// The user or a Mono stopped this turn. Native stops leave no transcript
    /// row, so completion reports read this after a reload or restart.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stopped: bool,
}
impl TurnInfo {
    pub fn has_duration(&self) -> bool {
        self.duration_known || self.duration_ms > 0
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Handoff {
    pub from: String,
    pub brief: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TurnMetrics {
    #[serde(alias = "inputTokens")]
    pub input_tokens: u64,
    #[serde(alias = "outputTokens")]
    pub output_tokens: u64,
    #[serde(alias = "cacheReadTokens")]
    pub cache_read_tokens: u64,
    #[serde(alias = "cacheWriteTokens")]
    pub cache_write_tokens: u64,
    #[serde(alias = "cacheHitPercent")]
    pub cache_hit_percent: Option<f64>,
}
impl TurnMetrics {
    pub fn reported(self) -> bool {
        self.input_tokens > 0
            || self.output_tokens > 0
            || self.cache_read_tokens > 0
            || self.cache_write_tokens > 0
            || self.cache_hit_percent.is_some()
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Approval {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub can_remember: bool,
    /// Whether "allow" is exactly one time. False when the provider offers
    /// no one-time option, so an allow would fall back to "always"; a remote
    /// device never answers such a request (U-13).
    pub can_allow_once: bool,
}
#[derive(Clone, Debug)]
pub enum Decision {
    Steer(crate::follow_up::Message),
    Allow,
    Session,
    Deny,
    Answer(Vec<Vec<String>>),
    KeepOpen,
}
#[derive(Clone, Debug, PartialEq)]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
    pub description: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct QuestionItem {
    pub id: String,
    pub prompt: String,
    pub multiple: bool,
    pub allow_custom: bool,
    pub options: Vec<QuestionOption>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    pub id: String,
    pub title: String,
    pub items: Vec<QuestionItem>,
    pub auto_resolve_at: Option<u64>,
}
pub struct Control {
    pub id: String,
    pub decision: Decision,
}
