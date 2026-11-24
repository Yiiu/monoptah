//! Per-agent configuration from Settings → Agents: the stored fields, the one-time
//! migration from the older "remember the last composer choice" preferences, and the
//! effective-config accessor (`Preferences::agent`) that every launch path reads.
//!
//! The older per-provider fields keep their JSON keys and meaning so the desktop
//! localStorage importer needs no change: `binary_paths` (command), `default_models`
//! (model; a missing key is the agent's default), `provider_surfaces` (used only when the
//! open mode is per agent), `hidden_providers` (disabled) and `default_provider`.
//! `default_access`, `provider_efforts` and `project_providers` stay in the data for older
//! builds but are read only by the migration.
use super::Preferences;
use monocode_core::{
    agent::{AccessMode, Permission},
    launch_args::{self, Launch},
    provider::Kind,
    surface::Surface,
};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;

pub(crate) const OPEN_CHAT: &str = "chat";
pub(crate) const OPEN_TUI: &str = "tui";
pub(crate) const OPEN_PER_AGENT: &str = "per-agent";

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub(crate) struct AgentPreferences {
    /// "chat" | "tui" | "per-agent". Empty only before the migration; it then reads as
    /// per agent, which is how sessions opened before it existed.
    pub open_mode: String,
    /// The global permission level id (`Permission::id`). `None` only before the
    /// migration; it then reads as the agent's own settings.
    pub permission: Option<String>,
    /// Per-agent permission override; a missing key follows `permission`.
    pub permissions: HashMap<String, String>,
    /// Per-agent effort; a missing key is the agent's default.
    pub efforts: HashMap<String, String>,
    /// Per-agent argument line as typed; it splits with shell rules (checked on save).
    pub args: HashMap<String, String>,
    /// Per-agent environment as `KEY=VALUE` lines (checked on save). Plain text by a
    /// documented decision; see `monocode_core::launch_args`.
    pub env: HashMap<String, String>,
}

/// Field by field, so a stray value in one field (from a newer or hand-edited file) is
/// dropped instead of failing the whole workspace, which would reset every preference.
impl<'de> Deserialize<'de> for AgentPreferences {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let map = |field: &str, valid: &dyn Fn(&str) -> bool| -> HashMap<String, String> {
            value[field]
                .as_object()
                .map(|entries| {
                    entries
                        .iter()
                        .filter(|(agent, _)| Kind::parse(agent).is_some())
                        .filter_map(|(agent, v)| Some((agent.clone(), v.as_str()?.to_owned())))
                        .filter(|(_, v)| valid(v))
                        .collect()
                })
                .unwrap_or_default()
        };
        let open_mode = value["open_mode"]
            .as_str()
            .filter(|mode| [OPEN_CHAT, OPEN_TUI, OPEN_PER_AGENT].contains(mode))
            .unwrap_or_default()
            .to_owned();
        let permission = value["permission"]
            .as_str()
            .filter(|id| Permission::parse(id).is_some())
            .map(str::to_owned);
        Ok(Self {
            open_mode,
            permission,
            permissions: map("permissions", &|id| Permission::parse(id).is_some()),
            efforts: map("efforts", &|effort| !effort.trim().is_empty()),
            args: map("args", &|_| true),
            env: map("env", &|_| true),
        })
    }
}

/// One agent's effective configuration: the global rules and per-agent overrides resolved.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AgentConfig {
    pub kind: Kind,
    /// CLI path override; `None` finds the CLI automatically.
    pub command: Option<String>,
    /// `None` is the agent's default (no model is passed).
    pub model: Option<String>,
    /// `None` is the agent's default.
    pub effort: Option<String>,
    /// The level this agent applies. An agent that cannot apply the configured level
    /// (`Permission::supported`) uses its own settings.
    pub permission: Permission,
    /// This agent has its own permission instead of following the global one.
    pub permission_overridden: bool,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Where a new local session opens; remote projects still open as chat.
    pub surface: Surface,
    /// Shown in pickers and allowed to be the default.
    pub enabled: bool,
}

impl AgentConfig {
    /// The access mode a new session of this agent starts with.
    pub(crate) fn access(&self) -> AccessMode {
        self.permission.access()
    }
    pub(crate) fn launch(&self) -> Launch {
        Launch {
            args: self.args.clone(),
            env: self.env.clone(),
        }
    }
}

fn level_for_access(id: &str) -> Permission {
    match id {
        "supervised" | "read-only" => Permission::Ask,
        "auto-accept-edits" => Permission::AcceptEdits,
        "auto" => Permission::Auto,
        "full-access" => Permission::Full,
        _ => Permission::Own,
    }
}

impl Preferences {
    /// The effective configuration of one agent. Launch paths, session seeding and the
    /// settings page read agent configuration only through this.
    pub(crate) fn agent(&self, kind: Kind) -> AgentConfig {
        let id = kind.id();
        let agents = &self.agents;
        let own = agents
            .permissions
            .get(id)
            .and_then(|p| Permission::parse(p));
        let configured = own.unwrap_or_else(|| self.agent_permission());
        let permission = if Permission::supported(kind).contains(&configured) {
            configured
        } else {
            Permission::Own
        };
        AgentConfig {
            kind,
            command: self
                .binary_paths
                .get(id)
                .filter(|path| !path.trim().is_empty())
                .cloned(),
            model: self
                .default_models
                .get(id)
                .filter(|model| !model.trim().is_empty() && model.as_str() != "default")
                .cloned(),
            effort: agents.efforts.get(id).cloned(),
            permission,
            permission_overridden: own.is_some(),
            args: agents
                .args
                .get(id)
                .and_then(|line| launch_args::split(line).ok())
                .unwrap_or_default(),
            env: agents
                .env
                .get(id)
                .and_then(|text| launch_args::parse_env(text).ok())
                .unwrap_or_default(),
            surface: self.agent_surface(kind),
            enabled: !self.hidden_providers.iter().any(|hidden| hidden == id),
        }
    }

    /// The permission level every agent follows unless it has its own.
    pub(crate) fn agent_permission(&self) -> Permission {
        self.agents
            .permission
            .as_deref()
            .and_then(Permission::parse)
            .unwrap_or(Permission::Own)
    }

    /// "chat", "tui" or "per-agent".
    pub(crate) fn open_mode(&self) -> &str {
        match self.agents.open_mode.as_str() {
            mode @ (OPEN_CHAT | OPEN_TUI) => mode,
            _ => OPEN_PER_AGENT,
        }
    }

    fn agent_surface(&self, kind: Kind) -> Surface {
        match self.open_mode() {
            OPEN_CHAT => Surface::Chat,
            OPEN_TUI => Surface::Tui,
            _ => self
                .provider_surfaces
                .get(kind.id())
                .copied()
                .unwrap_or_default(),
        }
    }

    /// Every agent's extra arguments and environment, for `monocode_core::cli::set_launches`.
    pub(crate) fn launches(&self) -> HashMap<String, Launch> {
        Kind::ALL
            .into_iter()
            .map(|kind| (kind.id().to_owned(), self.agent(kind).launch()))
            .filter(|(_, launch)| !launch.is_empty())
            .collect()
    }

    /// Set one agent's surface. Outside "per agent" this first switches to it, keeping
    /// every other agent where it opened before.
    pub(crate) fn set_agent_surface(&mut self, kind: Kind, surface: Surface) {
        if self.open_mode() != OPEN_PER_AGENT {
            for other in Kind::ALL {
                let current = self.agent_surface(other);
                self.provider_surfaces.insert(other.id().into(), current);
            }
            self.agents.open_mode = OPEN_PER_AGENT.into();
        }
        self.provider_surfaces.insert(kind.id().into(), surface);
    }

    /// Enable or disable an agent. A disabled agent cannot be the default, so disabling
    /// the default clears it (new sessions then fall back to the most recent choice, then
    /// Codex). Returns whether the default was cleared.
    pub(crate) fn set_agent_enabled(&mut self, kind: Kind, enabled: bool) -> bool {
        let id = kind.id();
        self.hidden_providers.retain(|hidden| hidden != id);
        if enabled {
            return false;
        }
        self.hidden_providers.push(id.into());
        if self.default_provider.as_deref() == Some(id) {
            self.default_provider = None;
            return true;
        }
        false
    }

    /// One-time move from the older preferences (`Workspace::restore` runs it while the
    /// saved state has no `agents.open_mode`). Seeds efforts from `provider_efforts`, the
    /// global permission from `default_access` (none: the agent's own settings), the open
    /// mode from the per-agent surfaces, and clears a default agent that is disabled.
    /// `project_providers` is not copied anywhere.
    pub(crate) fn migrate_agents(&mut self) {
        self.agents.efforts = self
            .provider_efforts
            .iter()
            .filter(|(agent, effort)| Kind::parse(agent).is_some() && !effort.trim().is_empty())
            .map(|(agent, effort)| (agent.clone(), effort.clone()))
            .collect();
        self.agents.permission = Some(
            self.default_access
                .as_deref()
                .map(level_for_access)
                .unwrap_or(Permission::Own)
                .id()
                .into(),
        );
        let surfaces: Vec<Surface> = Kind::ALL
            .into_iter()
            .map(|kind| {
                self.provider_surfaces
                    .get(kind.id())
                    .copied()
                    .unwrap_or_default()
            })
            .collect();
        self.agents.open_mode = if surfaces.iter().all(|s| *s == Surface::Chat) {
            OPEN_CHAT
        } else if surfaces.iter().all(|s| *s == Surface::Tui) {
            OPEN_TUI
        } else {
            OPEN_PER_AGENT
        }
        .into();
        if let Some(default) = self.default_provider.as_deref()
            && self.hidden_providers.iter().any(|hidden| hidden == default)
        {
            self.default_provider = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn migrated(saved: serde_json::Value) -> Preferences {
        let mut prefs: Preferences = serde_json::from_value(saved).unwrap();
        prefs.migrate_agents();
        prefs
    }

    #[test]
    fn migration_seeds_efforts_permission_open_mode_and_clears_a_disabled_default() {
        let prefs = migrated(json!({
            "provider_efforts": {"codex": "high", "claude": "max", "nobody": "low"},
            "default_access": "auto-accept-edits",
            "provider_surfaces": {"claude": "tui"},
            "default_provider": "codex",
            "hidden_providers": ["codex"],
            "project_providers": {"/work": {"provider": "claude", "hidden": ["grok"]}}
        }));
        assert_eq!(
            prefs.agents.efforts,
            HashMap::from([
                ("codex".to_string(), "high".to_string()),
                ("claude".to_string(), "max".to_string())
            ])
        );
        assert_eq!(prefs.agents.permission.as_deref(), Some("edits"));
        // Surfaces differ, so each agent keeps its own.
        assert_eq!(prefs.agents.open_mode, OPEN_PER_AGENT);
        assert_eq!(prefs.agent(Kind::Claude).surface, Surface::Tui);
        assert_eq!(prefs.agent(Kind::Codex).surface, Surface::Chat);
        assert_eq!(prefs.default_provider, None);
        // The old fields stay for older builds; project lists are not applied.
        assert_eq!(prefs.default_access.as_deref(), Some("auto-accept-edits"));
        assert!(prefs.agent(Kind::Grok).enabled);
    }

    #[test]
    fn migration_maps_each_access_mode_and_none_to_the_agents_own_settings() {
        for (access, level) in [
            (Some("supervised"), "ask"),
            (Some("read-only"), "ask"),
            (Some("auto-accept-edits"), "edits"),
            (Some("auto"), "auto"),
            (Some("full-access"), "full"),
            (None, "own"),
        ] {
            let prefs = migrated(json!({ "default_access": access }));
            assert_eq!(
                prefs.agents.permission.as_deref(),
                Some(level),
                "{access:?}"
            );
        }
    }

    #[test]
    fn uniform_surfaces_become_a_single_open_mode() {
        assert_eq!(migrated(json!({})).agents.open_mode, OPEN_CHAT);
        let all_tui: serde_json::Map<String, serde_json::Value> = Kind::ALL
            .into_iter()
            .map(|kind| (kind.id().to_string(), json!("tui")))
            .collect();
        let prefs = migrated(json!({ "provider_surfaces": all_tui }));
        assert_eq!(prefs.agents.open_mode, OPEN_TUI);
        assert_eq!(prefs.agent(Kind::Hermes).surface, Surface::Tui);
    }

    #[test]
    fn a_bad_value_in_one_agent_field_does_not_fail_the_preferences() {
        let prefs: Preferences = serde_json::from_value(json!({
            "sounds": false,
            "agents": {
                "open_mode": 7,
                "permission": "sometimes",
                "permissions": {"claude": "full", "codex": "bogus", "nobody": "ask"},
                "efforts": ["not", "a", "map"],
                "args": {"claude": "--verbose", "grok": 3},
                "env": "A=1"
            }
        }))
        .unwrap();
        assert!(!prefs.sounds);
        assert_eq!(prefs.agents.open_mode, "");
        assert_eq!(prefs.agents.permission, None);
        assert_eq!(
            prefs.agents.permissions,
            HashMap::from([("claude".to_string(), "full".to_string())])
        );
        assert!(prefs.agents.efforts.is_empty());
        assert_eq!(prefs.agents.args.len(), 1);
        assert!(prefs.agents.env.is_empty());
    }

    #[test]
    fn the_accessor_resolves_overrides_support_args_env_and_enabled() {
        let mut prefs = Preferences::default();
        prefs.agents.permission = Some("edits".into());
        prefs
            .agents
            .permissions
            .insert("codex".into(), "full".into());
        prefs
            .agents
            .args
            .insert("claude".into(), r#"--add-dir "a b""#.into());
        prefs.agents.args.insert("grok".into(), "\"unclosed".into());
        prefs
            .agents
            .env
            .insert("claude".into(), "HTTP_PROXY=http://127.0.0.1:9\n".into());
        prefs.default_models.insert("claude".into(), "opus".into());
        prefs
            .default_models
            .insert("codex".into(), "default".into());
        prefs
            .binary_paths
            .insert("claude".into(), "/opt/claude".into());
        prefs.hidden_providers.push("hermes".into());

        let claude = prefs.agent(Kind::Claude);
        assert_eq!(claude.permission, Permission::AcceptEdits);
        assert!(!claude.permission_overridden);
        assert_eq!(claude.access(), AccessMode::AutoAcceptEdits);
        assert_eq!(claude.args, ["--add-dir", "a b"]);
        assert_eq!(
            claude.env,
            [("HTTP_PROXY".to_string(), "http://127.0.0.1:9".to_string())]
        );
        assert_eq!(claude.model.as_deref(), Some("opus"));
        assert_eq!(claude.command.as_deref(), Some("/opt/claude"));

        let codex = prefs.agent(Kind::Codex);
        assert_eq!(codex.permission, Permission::Full);
        assert!(codex.permission_overridden);
        assert_eq!(codex.model, None, "\"default\" is the agent's default");
        // A line that cannot be split never reaches a launch.
        assert!(prefs.agent(Kind::Grok).args.is_empty());
        // Cursor cannot apply "accept edits"; Pi has no permission at all.
        assert_eq!(prefs.agent(Kind::Cursor).permission, Permission::Own);
        assert_eq!(prefs.agent(Kind::Pi).permission, Permission::Own);
        assert!(!prefs.agent(Kind::Hermes).enabled);

        let launches = prefs.launches();
        assert_eq!(launches.len(), 1);
        assert_eq!(launches["claude"].args, ["--add-dir", "a b"]);
    }

    #[test]
    fn open_mode_decides_the_surface_and_a_per_agent_edit_keeps_the_others() {
        let mut prefs = Preferences::default();
        prefs
            .provider_surfaces
            .insert("claude".into(), Surface::Tui);
        // Before the migration the per-agent surfaces apply.
        assert_eq!(prefs.agent(Kind::Claude).surface, Surface::Tui);
        prefs.agents.open_mode = OPEN_TUI.into();
        assert_eq!(prefs.agent(Kind::Codex).surface, Surface::Tui);
        prefs.agents.open_mode = OPEN_CHAT.into();
        assert_eq!(prefs.agent(Kind::Claude).surface, Surface::Chat);
        prefs.set_agent_surface(Kind::Grok, Surface::Tui);
        assert_eq!(prefs.open_mode(), OPEN_PER_AGENT);
        assert_eq!(prefs.agent(Kind::Grok).surface, Surface::Tui);
        assert_eq!(prefs.agent(Kind::Claude).surface, Surface::Chat);
    }

    #[test]
    fn disabling_the_default_agent_clears_the_default() {
        let mut prefs = Preferences {
            default_provider: Some("claude".into()),
            ..Default::default()
        };
        assert!(!prefs.set_agent_enabled(Kind::Codex, false));
        assert_eq!(prefs.default_provider.as_deref(), Some("claude"));
        assert!(prefs.set_agent_enabled(Kind::Claude, false));
        assert_eq!(prefs.default_provider, None);
        assert!(!prefs.agent(Kind::Claude).enabled);
        prefs.set_agent_enabled(Kind::Claude, true);
        assert!(prefs.agent(Kind::Claude).enabled);
        assert_eq!(prefs.hidden_providers, ["codex"]);
    }
}
