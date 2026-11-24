//! The provider and model a new conversation starts with. One resolver, so
//! every way of starting a chat agrees.
//!
//! Order: Settings default, last choice's provider (`recent_models[0]`), then Codex (or
//! the first allowed provider). The model is always the agent's Settings model. A step is
//! skipped when its provider is disabled. Per-project provider defaults are retired and no
//! longer apply. Hidden providers are ignored for remote projects, as the model picker
//! ignores them there.
use super::*;
use crate::settings::Preferences;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NewChatChoice {
    pub kind: Kind,
    pub model: Option<String>,
}

impl NewChatChoice {
    /// A provider picked by the caller: the model follows Settings.
    pub(crate) fn explicit(prefs: &Preferences, _cwd: &str, kind: Kind) -> Self {
        Self::of(kind, default_model(prefs, kind))
    }
    fn of(kind: Kind, model: Option<String>) -> Self {
        Self { kind, model }
    }
}

/// The agent's model from Settings → Agents; `None` is the agent's default.
fn default_model(prefs: &Preferences, kind: Kind) -> Option<String> {
    prefs.agent(kind).model
}

fn allowed(prefs: &Preferences, cwd: &str, kind: Kind) -> bool {
    monocode_core::remote::parse_remote_path(cwd).is_some()
        || !prefs.hidden_for(cwd).iter().any(|id| id == kind.id())
}

pub(crate) fn resolve(prefs: &Preferences, cwd: &str) -> NewChatChoice {
    // 1. The Settings provider.
    if let Some(kind) = prefs
        .default_provider
        .as_deref()
        .and_then(Kind::parse)
        .filter(|k| allowed(prefs, cwd, *k))
    {
        return NewChatChoice::of(kind, default_model(prefs, kind));
    }
    // 2. The last model picked anywhere.
    if let Some(recent) = prefs.recent_models.first()
        && let Some(kind) = Kind::parse(&recent.provider).filter(|k| allowed(prefs, cwd, *k))
    {
        // Only the provider: its model stays in that session (`None` is the agent's default).
        return NewChatChoice::of(kind, default_model(prefs, kind));
    }
    // 3. Codex, or the first provider left.
    let kind = [Kind::Codex]
        .into_iter()
        .chain(Kind::ALL)
        .find(|k| allowed(prefs, cwd, *k))
        .unwrap_or(Kind::Codex);
    NewChatChoice::of(kind, default_model(prefs, kind))
}

impl Shell {
    /// The choice for a new conversation in `cwd`.
    pub(super) fn new_chat_choice(&self, cwd: &str) -> NewChatChoice {
        resolve(&self.workspace.preferences, cwd)
    }
    /// A new chat in the current project.
    pub(super) fn default_choice(&self) -> NewChatChoice {
        self.new_chat_choice(&self.workspace.project)
    }
    /// A caller-picked provider in the current project.
    pub(super) fn explicit_choice(&self, kind: Kind) -> NewChatChoice {
        NewChatChoice::explicit(&self.workspace.preferences, &self.workspace.project, kind)
    }
}
