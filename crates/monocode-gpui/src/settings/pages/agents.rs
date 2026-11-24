//! The Agent CLIs groups of Settings → Providers: how new sessions open, the permission
//! every agent starts with, and one expandable row per agent holding its command, model,
//! reasoning effort, permission, arguments and environment.
//!
//! The rules live elsewhere: `Preferences::agent` resolves an agent's effective
//! configuration and `monocode_core::launch_args` parses arguments and environment. This
//! file only draws them and writes what the user picks.
use crate::{
    disclosure::Disclosure,
    input::{self, Input},
    popover::{
        Align, Anchor, AnchorSlot, Menu, MenuAction, MenuEvent, MenuItem, MenuStyle, Side,
        menu_popover,
    },
    settings::{
        AgentUpdateState, Preferences, SettingsEvent, SettingsView,
        preferences::{OPEN_CHAT, OPEN_PER_AGENT, OPEN_TUI},
        widgets::{
            RadioCard, accent, badge, chip, code_block, env_row, env_table, field, field_error,
            help, icon_action, icon_tile, inline_banner, note, saved_flash, secondary_button,
            settings_toast, skeleton_row, status_line, text_area, text_box, tinted,
        },
    },
    theme::*,
};
use gpui::{
    AnyElement, ClipboardItem, Context, Entity, Focusable, FontWeight, Role, SharedString,
    Subscription, Task, Window, div, prelude::*, px,
};
use monocode_core::{
    agent::Permission, cli::Inspection, launch_args, provider::Kind, surface::Surface,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant},
};

/// Shown instead of the open-mode cards when the database cannot store a terminal session.
const NO_TERMINAL_HINT: &str = "Terminal sessions need a newer database.";
/// The model select's value for "type a model name".
const CUSTOM_MODEL: &str = "__custom";
const SAVED_FLASH: Duration = Duration::from_millis(1400);
const TOAST_TIME: Duration = Duration::from_secs(5);
/// The group card's `rounded_xl` less its 1px border: the radius a row's fill needs at the
/// card's first and last row, because clipping in GPUI is rectangular.
const CARD_INNER_RADIUS: f32 = 11.;
/// The fill of an open row, shared by its header and its panel so they read as one surface.
const OPEN_FILL: f32 = 0.03;
/// What hovering a row's header adds: Settings' interaction ink.
const HOVER_FILL: f32 = 0.05;

/// Checks an agent's CLI: `monocode_core::cli::inspect`, or a stand-in set by tests so
/// none of them starts a CLI installed on the machine.
pub(in crate::settings) type Inspector =
    std::sync::Arc<dyn Fn(Kind, Option<&str>) -> Result<Inspection, String> + Send + Sync>;

/// A field of an agent's expanded row, for its error and "Saved" mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::settings) enum Field {
    Surface,
    Permission,
    Command,
    Model,
    Effort,
    Args,
    Env,
}

/// What an open menu belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum MenuTarget {
    /// An agent row's "more actions" menu.
    Row(Kind),
    /// The permission every agent follows.
    GlobalPermission,
    /// One agent's permission.
    Permission(Kind),
}

struct OpenMenu {
    target: MenuTarget,
    menu: Entity<Menu>,
    _events: Subscription,
}

/// The text fields of the expanded row. They exist only while it is open.
struct Inputs {
    command: Entity<Input>,
    model: Entity<Input>,
    args: Entity<Input>,
    env: Entity<Input>,
    /// The saved value each field last showed. A field follows the saved value only when
    /// that changes: the frame drawn after focus leaves a field comes before its blur
    /// listener, and must not put the typed text back to the saved one.
    seen: HashMap<Field, String>,
    _subscriptions: Vec<Subscription>,
}

/// Takes one change back: it restores only what that change wrote.
type Undo = Box<dyn FnOnce(&mut Preferences)>;

/// The notice at the bottom right, with what its Undo does.
pub(in crate::settings) struct Toast {
    glyph: &'static str,
    text: SharedString,
    undo: Option<Undo>,
}

#[derive(Default)]
pub(in crate::settings) struct AgentsState {
    /// The one expanded row.
    pub(in crate::settings) expanded: Option<Kind>,
    /// Each row's panel: open, closed or on its way. A closing row stays here (and keeps
    /// its text fields) until it has shut.
    disclosures: HashMap<Kind, Disclosure>,
    menu: Option<OpenMenu>,
    anchors: RefCell<HashMap<MenuTarget, AnchorSlot>>,
    /// The text fields of every row whose panel is on screen: the open one, and one that
    /// is still closing.
    inputs: HashMap<Kind, Inputs>,
    errors: HashMap<(Kind, Field), String>,
    saved: Option<(Kind, Field)>,
    _saved: Option<Task<()>>,
    /// Secret variables whose value is on show.
    revealed: HashSet<(Kind, String)>,
    editing_env: Option<Kind>,
    /// The agent whose model is being typed instead of picked.
    custom_model: Option<Kind>,
    /// Each CLI's resolved path and version, or why it could not be checked.
    /// Only a result for the command in effect is kept: a check of a command that has
    /// since been replaced is dropped when it lands.
    pub(in crate::settings) inspections: HashMap<Kind, Result<Inspection, String>>,
    /// How many checks of each agent's CLI are under way, of either kind.
    inspecting: HashMap<Kind, usize>,
    /// The command each update offer on screen was found for. An offer belongs to the CLI
    /// that was checked: once the command changes, the row asks for a new check instead
    /// of running that offer's update against another installation.
    offer_commands: HashMap<Kind, Option<String>>,
    /// The latest background version check of each agent, and separately the latest check
    /// of a command being saved: a refresh must not supersede a save that is under way.
    inspect_generation: HashMap<Kind, u64>,
    save_generation: HashMap<Kind, u64>,
    /// `None` checks the real CLI.
    pub(in crate::settings) inspector: Option<Inspector>,
    pub(in crate::settings) toast: Option<Toast>,
    _toast: Option<Task<()>>,
    /// When installed agents were last detected.
    checked_at: Option<Instant>,
}

/// The search and scroll target of an agent's row.
pub(in crate::settings) fn agent_row_id(kind: Kind) -> &'static str {
    match kind {
        Kind::Codex => "settings-agent-codex",
        Kind::Claude => "settings-agent-claude",
        Kind::Cursor => "settings-agent-cursor",
        Kind::Grok => "settings-agent-grok",
        Kind::OpenCode => "settings-agent-opencode",
        Kind::Antigravity => "settings-agent-antigravity",
        Kind::Pi => "settings-agent-pi",
        Kind::Omp => "settings-agent-omp",
        Kind::Fx => "settings-agent-fx",
        Kind::Hermes => "settings-agent-hermes",
        Kind::Devin => "settings-agent-devin",
    }
}

/// The agent a search result lands on, when it is an agent's row.
pub(in crate::settings) fn agent_for_setting(id: &str) -> Option<Kind> {
    Kind::ALL.into_iter().find(|kind| agent_row_id(*kind) == id)
}

/// `select` keys its open state and anchor by a static id.
fn model_select_id(kind: Kind) -> &'static str {
    match kind {
        Kind::Codex => "agent-model-codex",
        Kind::Claude => "agent-model-claude",
        Kind::Cursor => "agent-model-cursor",
        Kind::Grok => "agent-model-grok",
        Kind::OpenCode => "agent-model-opencode",
        Kind::Antigravity => "agent-model-antigravity",
        Kind::Pi => "agent-model-pi",
        Kind::Omp => "agent-model-omp",
        Kind::Fx => "agent-model-fx",
        Kind::Hermes => "agent-model-hermes",
        Kind::Devin => "agent-model-devin",
    }
}

fn surface_name(surface: Surface) -> &'static str {
    if surface.is_tui() {
        "Terminal (TUI)"
    } else {
        "Chat"
    }
}

/// The command a session of `kind` runs: the configured path, else the CLI's name.
fn command_name(kind: Kind, command: Option<&str>, terminal: bool) -> String {
    // The Antigravity terminal is always `agy` from the PATH (`tui_launch::tui_binary`):
    // its configured command is the ACP server chat sessions talk to, not a terminal UI.
    if kind == Kind::Antigravity && terminal {
        return "agy".into();
    }
    match command {
        Some(path) => path.to_owned(),
        None => kind.binary_name().into(),
    }
}

/// The arguments `parts` had that `rest` no longer has, in their order and with their
/// boundaries: what `launch_args::extract_structured` moved out of an argument list.
fn moved_tokens(parts: &[String], rest: &[String]) -> Vec<String> {
    let mut kept: HashMap<&str, usize> = HashMap::new();
    for arg in rest {
        *kept.entry(arg.as_str()).or_default() += 1;
    }
    parts
        .iter()
        .filter(|arg| match kept.get_mut(arg.as_str()) {
            Some(count) if *count > 0 => {
                *count -= 1;
                false
            }
            _ => true,
        })
        .cloned()
        .collect()
}

/// The flag line a permission level passes to this agent, for the picker's rows.
fn permission_detail(kind: Kind, level: Permission, terminal: bool) -> String {
    let flags = launch_args::structured_flags(kind, terminal, None, None, level.access());
    if !flags.is_empty() {
        return launch_args::join(&flags);
    }
    if level == Permission::Own {
        "no flag".into()
    } else if terminal {
        "not passed to terminal sessions".into()
    } else {
        "set over the agent's protocol".into()
    }
}

/// Put a per-agent value back to what it was: the value, or no entry.
fn restore(map: &mut HashMap<String, String>, id: &str, before: Option<String>) {
    match before {
        Some(value) => {
            map.insert(id.into(), value);
        }
        None => {
            map.remove(id);
        }
    }
}

fn checked_ago(at: Instant) -> String {
    let minutes = at.elapsed().as_secs() / 60;
    match minutes {
        0 => "Checked just now".into(),
        1 => "Checked 1 min ago".into(),
        m if m < 60 => format!("Checked {m} min ago"),
        m => format!("Checked {} h ago", m / 60),
    }
}

impl SettingsView {
    /// The path of an installed agent's CLI: what detection found, else a path the user
    /// entered that checked out.
    fn agent_path(&self, kind: Kind) -> Option<PathBuf> {
        self.discovery
            .binaries
            .iter()
            .find(|(k, _)| *k == kind)
            .and_then(|(_, path)| path.clone())
            .or_else(|| {
                self.agents
                    .inspections
                    .get(&kind)
                    .and_then(|result| result.as_ref().ok())
                    .map(|inspection| inspection.path.clone())
            })
    }

    /// The CLI the agent runs now, for "Open CLI location": the checked command (a saved
    /// override that passed), else what detection found.
    pub(in crate::settings) fn active_cli_path(&self, kind: Kind) -> Option<PathBuf> {
        self.agents
            .inspections
            .get(&kind)
            .and_then(|result| result.as_ref().ok())
            .map(|inspection| inspection.path.clone())
            .or_else(|| self.agent_path(kind))
    }

    /// Whether a new session of `kind` opens in a terminal: the agent's surface as session
    /// creation resolves it for the selected project (a project on another machine opens
    /// as chat), and never when the database cannot store a terminal session.
    fn launches_in_terminal(&self, kind: Kind) -> bool {
        !self.ctx.no_terminal_sessions
            && monocode_core::surface::new_session_surface(
                kind,
                &self.selected_project,
                self.prefs.agent(kind).surface,
            )
            .is_tui()
    }

    /// The arguments a new session of `kind` starts with, in the order the launch passes
    /// them, split around the permission flags so the preview can tint those: what comes
    /// before them, the permission flags, and what comes after. The list itself comes from
    /// core: `tui_launch::structured_flags` for a terminal, `provider::chat_launch_args`
    /// for a chat.
    fn agent_launch_args(&self, kind: Kind) -> (Vec<String>, Vec<String>, Vec<String>) {
        let agent = self.prefs.agent(kind);
        let terminal = self.launches_in_terminal(kind);
        let mut argv = if terminal {
            let mut argv = launch_args::structured_flags(
                kind,
                true,
                agent.model.as_deref(),
                agent.effort.as_deref(),
                agent.access(),
            );
            argv.extend(agent.args.iter().cloned());
            argv
        } else {
            monocode_core::provider::chat_launch_args(
                kind,
                agent.model.as_deref(),
                agent.effort.as_deref(),
                agent.access(),
                &agent.args,
            )
        };
        let permission = launch_args::structured_flags(kind, terminal, None, None, agent.access());
        // The permission flags sit together somewhere among the others.
        let at = (!permission.is_empty())
            .then(|| {
                argv.windows(permission.len())
                    .position(|window| window == permission.as_slice())
            })
            .flatten();
        match at {
            Some(at) => {
                let rest = argv.split_off(at + permission.len());
                argv.truncate(at);
                (argv, permission, rest)
            }
            None => (argv, Vec::new(), Vec::new()),
        }
    }

    /// The launch command as one line; `mask` hides secret values that are not revealed.
    fn agent_launch_line(&self, kind: Kind, mask: bool) -> String {
        let agent = self.prefs.agent(kind);
        let terminal = self.launches_in_terminal(kind);
        let (lead, permission, rest) = self.agent_launch_args(kind);
        let mut parts: Vec<String> = agent
            .env
            .iter()
            .map(|(key, value)| {
                let hidden = mask
                    && launch_args::is_secret(key)
                    && !self.agents.revealed.contains(&(kind, key.clone()));
                if hidden {
                    format!("{key}=••••")
                } else {
                    format!("{key}={}", launch_args::join(std::slice::from_ref(value)))
                }
            })
            .collect();
        parts.push(launch_args::join(&[command_name(
            kind,
            agent.command.as_deref(),
            terminal,
        )]));
        parts.extend(
            lead.iter()
                .chain(&permission)
                .chain(&rest)
                .map(|arg| launch_args::join(std::slice::from_ref(arg))),
        );
        parts.join(" ")
    }

    /// The Agent CLIs groups: defaults, installed agents and the ones available to install.
    pub(super) fn agent_groups(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        // A panel on screen (open, or still closing) has its text fields; one that has
        // shut gives them up.
        let now = Instant::now();
        self.agents
            .disclosures
            .retain(|_, disclosure| disclosure.mounted(now));
        let mounted: Vec<Kind> = self.agents.disclosures.keys().copied().collect();
        self.agents.inputs.retain(|kind, _| mounted.contains(kind));
        for kind in &mounted {
            self.ensure_agent_inputs(*kind, window, cx);
        }
        if self
            .agents
            .disclosures
            .values()
            .any(|disclosure| disclosure.animating(now))
        {
            window.request_animation_frame();
        }
        self.bind_update_offers();
        let (installed, available): (Vec<Kind>, Vec<Kind>) = Kind::ALL
            .into_iter()
            .partition(|kind| self.agent_path(*kind).is_some());

        let mut defaults = vec![self.row(
            Some("agent-open-mode"),
            true,
            "Open new sessions in",
            Some(if self.ctx.no_terminal_sessions {
                format!("Where a new session with an agent starts. {NO_TERMINAL_HINT}").into()
            } else {
                "Where a new session with an agent starts.".into()
            }),
            Vec::new(),
        )];
        if !self.ctx.no_terminal_sessions {
            defaults.push(
                div()
                    .px_4()
                    .pb_4()
                    .child(self.open_mode_cards(cx))
                    .into_any_element(),
            );
        }
        let global = self.prefs.agent_permission();
        defaults.push(
            self.row(
                Some("agent-permission"),
                false,
                "Permissions",
                Some(
                    "What every agent may do without asking. You can set one agent differently below."
                        .into(),
                ),
                vec![self.permission_trigger(MenuTarget::GlobalPermission, global, None, cx)],
            ),
        );
        if global == Permission::Full {
            defaults.push(
                div()
                    .px_4()
                    .pb_4()
                    .child(inline_banner(
                        "agents-full-access-warning",
                        "shield-off",
                        permission_color(Permission::Full),
                        permission_color(Permission::Full),
                        "Agents will edit files and run commands without asking. Use it only in folders you trust.",
                        None,
                    ))
                    .into_any_element(),
            );
        }

        let mut installed_rows = Vec::new();
        if self.discovery_loading && installed.is_empty() {
            installed_rows.push(
                div()
                    .id("agent-detecting")
                    .debug_selector(|| "settings-agent-detecting".into())
                    .role(Role::Status)
                    .aria_label("Detecting installed agents")
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py(px(14.))
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(content(0.5))
                    .child(crate::right_panel::spinner(
                        "agent-detection-spinner",
                        "loader",
                        14.,
                    ))
                    .child("Detecting installed agents…")
                    .into_any_element(),
            );
            installed_rows.extend((0..3).map(|_| skeleton_row()));
        } else if installed.is_empty() {
            installed_rows.push(note(
                "No agent CLI was found. Install one below, or enter its path in its row.",
            ));
        }
        let lead = installed_rows.len();
        for (index, kind) in installed.iter().enumerate() {
            installed_rows.push(self.agent_row(
                *kind,
                true,
                lead + index == 0,
                index + 1 == installed.len(),
                cx,
            ));
        }
        let available_rows: Vec<AnyElement> = available
            .iter()
            .enumerate()
            .map(|(index, kind)| {
                self.agent_row(*kind, false, index == 0, index + 1 == available.len(), cx)
            })
            .collect();

        let counted = |title: &'static str, count: String| {
            div().flex().items_center().gap_2().child(title).child(
                div()
                    .font_family(crate::fonts::mono())
                    .text_size(px(11.))
                    .font_weight(FontWeight::NORMAL)
                    .text_color(content(0.45))
                    .child(count),
            )
        };
        let refresh = div()
            .flex()
            .items_center()
            .gap_3()
            .when_some(self.agents.checked_at, |d, at| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .line_height(px(16.))
                        .text_color(content(0.45))
                        .child(checked_ago(at)),
                )
            })
            .child(self.refresh_button("refresh-agents", "Refresh", cx))
            .into_any_element();
        let mut groups = vec![
            self.group(
                Some("agent-clis"),
                false,
                "Agent CLIs",
                Some(
                    "Detect, install and configure every agent. Changes apply to new sessions."
                        .into(),
                ),
                None,
                defaults,
            ),
            self.group(
                None,
                false,
                counted("Installed", format!("{} detected", installed.len())),
                None,
                Some(refresh),
                installed_rows,
            ),
        ];
        if !available.is_empty() {
            groups.push(self.group(
                None,
                false,
                counted(
                    "Available to install",
                    format!(
                        "{} {}",
                        available.len(),
                        if available.len() == 1 {
                            "agent"
                        } else {
                            "agents"
                        }
                    ),
                ),
                None,
                None,
                available_rows,
            ));
        }
        groups
    }

    /// "Open new sessions in": Chat, Terminal (TUI) or Choose per agent.
    fn open_mode_cards(&self, cx: &mut Context<Self>) -> AnyElement {
        let bubble = |width: f32, mine: bool| {
            div()
                .h(px(8.))
                .w(gpui::relative(width))
                .rounded(px(4.))
                .map(|d| {
                    if mine {
                        d.ml_auto().bg(accent(0.45))
                    } else {
                        d.bg(content(0.16))
                    }
                })
        };
        let chat = |a: f32, b: f32| {
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap_1()
                .child(bubble(a, false))
                .child(bubble(b, true))
        };
        let terminal = |lines: &[&'static str]| {
            div()
                .flex_1()
                .font_family(crate::fonts::mono())
                .text_size(px(10.))
                .line_height(px(12.))
                .text_color(content(0.55))
                .children(lines.iter().map(|line| div().truncate().child(*line)))
        };
        let cards = vec![
            RadioCard {
                value: OPEN_CHAT,
                title: "Chat".into(),
                description: "Chat in Monoptah".into(),
                glyph: None,
                preview: Some(chat(0.6, 0.45).child(bubble(0.7, false)).into_any_element()),
            },
            RadioCard {
                value: OPEN_TUI,
                title: "Terminal (TUI)".into(),
                description: "The agent's own terminal UI".into(),
                glyph: None,
                preview: Some(
                    terminal(&["$ claude", "  Thinking…", "  Edited 2 files"]).into_any_element(),
                ),
            },
            RadioCard {
                value: OPEN_PER_AGENT,
                title: "Choose per agent".into(),
                description: "Set it on each agent below".into(),
                glyph: None,
                preview: Some(
                    div()
                        .flex()
                        .gap_2()
                        .child(chat(0.8, 0.6))
                        .child(terminal(&["$ codex"]))
                        .into_any_element(),
                ),
            },
        ];
        self.radio_cards(
            "agent-open-mode",
            "Open new sessions in",
            self.prefs.open_mode(),
            cards,
            false,
            cx,
            // Only the mode changes: each agent's own surface stays saved, so coming back
            // to "per agent" finds every row as it was left.
            |this, value, cx| this.update(cx, |p| p.agents.open_mode = value.into()),
        )
    }

    /// The permission picker's trigger: the select trigger with the level's tinted icon.
    /// `inherited` is the level an agent follows while it has none of its own.
    fn permission_trigger(
        &self,
        target: MenuTarget,
        level: Permission,
        inherited: Option<Permission>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open = self
            .agents
            .menu
            .as_ref()
            .is_some_and(|menu| menu.target == target);
        let slot = self
            .agents
            .anchors
            .borrow_mut()
            .entry(target)
            .or_default()
            .clone();
        let shown = inherited.unwrap_or(level);
        let name = match target {
            MenuTarget::Permission(kind) => kind.id(),
            _ => "global",
        };
        div()
            .id(SharedString::from(format!("permission-{name}")))
            .debug_selector(move || format!("settings-permission-{name}"))
            .role(Role::ComboBox)
            .aria_label(format!("Permissions: {}", shown.label()))
            .aria_expanded(open)
            .relative()
            .w(px(248.))
            .max_w_full()
            .flex()
            .items_center()
            .gap_2()
            .rounded_md()
            .border_1()
            .border_color(if shown == Permission::Full {
                tinted(permission_color(Permission::Full), 0.5)
            } else {
                content(0.1)
            })
            .bg(content(0.05))
            .pl_1()
            .pr_2()
            .py(px(3.))
            .text_size(px(12.))
            .line_height(px(16.))
            .text_color(content(1.))
            .cursor_pointer()
            .hover(|s| s.border_color(content(0.2)))
            .child(slot.probe())
            .child(icon_tile(
                permission_icon(shown),
                permission_color(shown),
                20.,
            ))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(if inherited.is_some() {
                        format!("Same as all agents · {}", shown.label())
                    } else {
                        shown.label().to_owned()
                    }),
            )
            .child(icon("chevrons-up-down", 14.).text_color(content(0.5)))
            .on_click(
                cx.listener(move |this, _, window, cx| this.toggle_agent_menu(target, window, cx)),
            )
            .into_any_element()
    }

    /// One agent: its collapsed header and, when it is the expanded row, its fields.
    fn agent_row(
        &self,
        kind: Kind,
        installed: bool,
        first: bool,
        last: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = Instant::now();
        let disclosure = self.agents.disclosures.get(&kind);
        // How far open the row is: it turns the chevron and fades the open fill in.
        let fraction = disclosure.map_or(0., |disclosure| disclosure.fraction(now));
        let agent = self.prefs.agent(kind);
        let open = self.agents.expanded == Some(kind);
        let is_default = self.prefs.default_provider.as_deref() == Some(kind.id());
        let per_agent = self.prefs.open_mode() == OPEN_PER_AGENT;
        let id = agent_row_id(kind);
        let dim = installed && !agent.enabled;

        let mut badges: Vec<AnyElement> = Vec::new();
        if is_default {
            badges.push(badge("Default", None, accent_color()).into_any_element());
        }
        if dim {
            badges.push(badge("Off", None, content(0.5)).into_any_element());
        }
        // An agent that follows the default shows no badge: the default sits above.
        if installed && agent.permission_overridden && !Permission::supported(kind).is_empty() {
            badges.push(
                badge(
                    agent.permission.label(),
                    Some(permission_icon(agent.permission)),
                    permission_color(agent.permission),
                )
                .debug_selector(move || format!("settings-agent-permission-badge-{}", kind.id()))
                .into_any_element(),
            );
        }
        let summary = if installed {
            // The launch command without its variables, which the summary only counts.
            let (lead, permission, rest) = self.agent_launch_args(kind);
            let terminal = self.launches_in_terminal(kind);
            let mut parts = vec![command_name(kind, agent.command.as_deref(), terminal)];
            parts.extend(lead.into_iter().chain(permission).chain(rest));
            let mut text = launch_args::join(&parts);
            if !agent.env.is_empty() {
                text.push_str(&format!("  · {} env", agent.env.len()));
            }
            text
        } else {
            kind.install_command()
                .map(str::to_owned)
                .unwrap_or_else(|| "Not found on your PATH".into())
        };

        let header = div()
            .id(SharedString::from(format!("agent-header-{}", kind.id())))
            .debug_selector(move || format!("settings-agent-header-{}", kind.id()))
            .role(Role::Button)
            .aria_label(format!("{} settings", kind.label()))
            .aria_expanded(open)
            .min_w_0()
            .flex_1()
            .flex()
            .items_center()
            .gap_3()
            // The header reaches the card's left edge and is as tall as the row, so the
            // whole strip left of the controls opens it.
            .pl_3()
            .py_2()
            .cursor_pointer()
            .child({
                icon("chevron-right", 14.)
                    .text_color(content(0.4))
                    .with_transformation(gpui::Transformation::rotate(gpui::percentage(
                        0.25 * fraction,
                    )))
            })
            .child(
                div()
                    .size(px(28.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.))
                    .bg(content(0.06))
                    .when(dim || !installed, |d| d.opacity(0.55))
                    .child(provider_icon(kind.id(), 16.)),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(13.))
                            .line_height(px(19.5))
                            .font_weight(FontWeight::MEDIUM)
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(content(if dim {
                                        0.45
                                    } else if installed {
                                        1.
                                    } else {
                                        0.7
                                    }))
                                    .child(kind.label()),
                            )
                            .children(badges),
                    )
                    .child(
                        div()
                            .debug_selector(move || format!("settings-agent-summary-{}", kind.id()))
                            .mt(px(2.))
                            .truncate()
                            .font_family(crate::fonts::mono())
                            .text_size(px(11.))
                            .line_height(px(16.))
                            .text_color(content(if dim { 0.3 } else { 0.45 }))
                            .child(summary),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_expanded_agent((!open).then_some(kind), cx)
            }));

        let mut controls: Vec<AnyElement> = Vec::new();
        if installed {
            controls.push(self.agent_version(kind, cx));
            if per_agent && !self.ctx.no_terminal_sessions {
                controls.push(self.segmented_icons(
                    format!("agent-surface-{}", kind.id()),
                    format!("{} opens in", kind.label()),
                    agent.surface.id(),
                    &[
                        (Surface::Chat.id(), "Chat", Some("message-square")),
                        (Surface::Tui.id(), "TUI", Some("terminal")),
                    ],
                    cx,
                    move |this, value, cx| {
                        if let Some(surface) = Surface::parse(value) {
                            this.update(cx, |p| p.set_agent_surface(kind, surface))
                        }
                    },
                ));
            }
            controls.push(self.toggle(
                kind.id(),
                format!("Use {}", kind.label()),
                agent.enabled,
                false,
                cx,
                move |this, on, cx| this.set_agent_enabled(kind, on, cx),
            ));
        } else {
            controls.push(
                secondary_button(
                    SharedString::from(format!("install-{}", kind.id())),
                    format!("Install {}", kind.label()),
                    false,
                    false,
                )
                .debug_selector(move || format!("settings-agent-install-{}", kind.id()))
                .child(icon("download", 12.))
                .child("Install")
                .on_click(move |_, _, cx| cx.open_url(kind.docs_url()))
                .into_any_element(),
            );
        }
        let slot = self
            .agents
            .anchors
            .borrow_mut()
            .entry(MenuTarget::Row(kind))
            .or_default()
            .clone();
        controls.push(
            icon_action(
                SharedString::from(format!("agent-menu-{}", kind.id())),
                format!("More actions for {}", kind.label()),
                "more-horizontal",
                28.,
                16.,
                0.6,
                false,
            )
            .debug_selector(move || format!("settings-agent-menu-{}", kind.id()))
            .relative()
            .child(slot.probe())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.toggle_agent_menu(MenuTarget::Row(kind), window, cx)
            }))
            .into_any_element(),
        );

        let panel = disclosure
            .and_then(|disclosure| disclosure.render(self.agent_panel(kind, installed, cx), now));
        // The card clips to a rectangle, so a fill rounds its own corners where the row
        // meets the card's: the top of the first row, the bottom of the last.
        let corners = |d: gpui::Div, bottom: bool| {
            d.when(first, |d| d.rounded_t(px(CARD_INNER_RADIUS)))
                .when(last && bottom, |d| d.rounded_b(px(CARD_INNER_RADIUS)))
        };
        // One fill behind the header and the panel of an open row; the header adds only
        // its hover on top, and the panel has no fill or hover of its own.
        corners(div(), true)
            .debug_selector(move || id.into())
            .relative()
            .when(!first, |d| d.border_t_1().border_color(content(0.05)))
            .when(fraction > 0., |d| d.bg(content(OPEN_FILL * fraction)))
            .child(self.reveal_probe(id))
            .child(
                // Stretching (no `items_center` here) makes the header as tall as the strip.
                // GPUI keeps hover state per element id: without one, moving the pointer
                // never redraws the strip, and its hover shows only when something else
                // happens to repaint the page.
                corners(div(), panel.is_none())
                    .id(SharedString::from(format!("agent-strip-{}", kind.id())))
                    .relative()
                    .min_h(px(56.))
                    .flex()
                    .map(|d| {
                        if self.flashing(Some(id)) {
                            d.bg(accent(0.1))
                        } else {
                            // On the strip, so it holds while the pointer is over the
                            // switch or a button inside it.
                            d.hover(|s| s.bg(content(HOVER_FILL)))
                        }
                    })
                    .child(header)
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_3()
                            .pl_3()
                            .pr_3()
                            .children(controls),
                    ),
            )
            .children(panel)
            .into_any_element()
    }

    /// The version on the right of an installed agent's row, or its update button.
    fn agent_version(&self, kind: Kind, cx: &mut Context<Self>) -> AnyElement {
        let version = |text: String| {
            div()
                .flex_shrink_0()
                .font_family(crate::fonts::mono())
                .text_size(px(11.))
                .line_height(px(16.))
                .text_color(content(0.45))
                .whitespace_nowrap()
                .child(text)
                .into_any_element()
        };
        let update = self
            .ctx
            .agent_updates
            .iter()
            .find(|update| update.kind == kind);
        match update.map(|update| (&update.state, update.latest.clone())) {
            Some((AgentUpdateState::Updating, _)) => div()
                .flex()
                .items_center()
                .gap_1()
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(content(0.5))
                .child(crate::right_panel::spinner(
                    SharedString::from(format!("agent-updating-{}", kind.id())),
                    "loader",
                    12.,
                ))
                .child("Updating…")
                .into_any_element(),
            Some((AgentUpdateState::Updated(installed), _)) => version(format!("v{installed}")),
            // The offer was found for a command that has since changed: check the CLI in
            // effect now rather than update another installation.
            Some(_) if self.update_offer_stale(kind) => secondary_button(
                SharedString::from(format!("recheck-update-{}", kind.id())),
                format!("Check for a {} update", kind.label()),
                false,
                false,
            )
            .debug_selector(move || format!("settings-agent-update-check-{}", kind.id()))
            .child(icon("refresh-cw", 12.))
            .child("Check for update")
            .on_click(cx.listener(move |this, _, _, cx| this.recheck_update_offer(kind, cx)))
            .into_any_element(),
            Some((state, latest)) => {
                let failed = matches!(state, AgentUpdateState::Failed(_));
                let label = if failed {
                    format!("Retry update to {latest}")
                } else {
                    format!("Update to {latest}")
                };
                secondary_button(
                    SharedString::from(format!("update-agent-{}", kind.id())),
                    format!("{label} for {}", kind.label()),
                    false,
                    false,
                )
                .debug_selector(move || format!("settings-agent-update-{}", kind.id()))
                .border_color(accent(0.4))
                .bg(accent(0.08))
                .text_color(accent_color())
                .child(icon("arrow-up", 12.).text_color(accent_color()))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.update_offer_stale(kind) {
                        this.recheck_update_offer(kind, cx);
                    } else {
                        cx.emit(SettingsEvent::UpdateAgent(kind));
                    }
                }))
                .into_any_element()
            }
            None => match self.agents.inspections.get(&kind) {
                Some(Ok(inspection)) => version(
                    monocode_core::harness_updates::parse_version(&inspection.version)
                        .map(|v| format!("v{v}"))
                        .unwrap_or_else(|| inspection.version.clone()),
                ),
                _ => div().into_any_element(),
            },
        }
    }

    /// The expanded row's fields. An agent that is not installed shows only how to install
    /// it and where its CLI is.
    fn agent_panel(&self, kind: Kind, installed: bool, cx: &mut Context<Self>) -> AnyElement {
        let Some(inputs) = self.agents.inputs.get(&kind) else {
            return div().into_any_element();
        };
        let agent = self.prefs.agent(kind);
        let saved = |field: Field| (self.agents.saved == Some((kind, field))).then(saved_flash);
        let error = |field: Field, name: &'static str| {
            self.agents.errors.get(&(kind, field)).map(|text| {
                field_error(
                    SharedString::from(format!("agent-error-{name}-{}", kind.id())),
                    format!("settings-agent-error-{name}-{}", kind.id()),
                    text.clone(),
                )
            })
        };
        let mut fields: Vec<AnyElement> = Vec::new();
        let command_error = error(Field::Command, "command");
        let command_field = |first: bool,
                             hint: Option<SharedString>,
                             notes: Vec<AnyElement>,
                             cx: &mut Context<Self>| {
            let override_set = agent.command.is_some();
            let path = self.active_cli_path(kind);
            let mut line = div().flex().items_center().gap_2().child(
                text_box(inputs.command.clone(), true, command_error.is_some())
                    .debug_selector(move || format!("settings-agent-command-{}", kind.id())),
            );
            if override_set {
                line = line.child(
                    secondary_button(
                        SharedString::from(format!("reset-command-{}", kind.id())),
                        "Use the auto-detected path",
                        false,
                        false,
                    )
                    .border_0()
                    .child("Reset")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_agent_command(kind, String::new(), cx)
                    })),
                );
            }
            line = line
                .child(
                    icon_action(
                        SharedString::from(format!("reveal-cli-{}", kind.id())),
                        "Open CLI location",
                        "folder-open",
                        28.,
                        14.,
                        0.6,
                        path.is_none(),
                    )
                    .on_click(move |_, _, cx| {
                        if let Some(path) = &path {
                            cx.reveal_path(path);
                        }
                    }),
                )
                .child(
                    icon_action(
                        SharedString::from(format!("check-cli-{}", kind.id())),
                        "Check version",
                        "refresh-cw",
                        28.,
                        14.,
                        0.6,
                        self.agents.inspecting.contains_key(&kind),
                    )
                    .debug_selector(move || format!("settings-agent-check-{}", kind.id()))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.check_agent_command(kind, cx)),
                    ),
                );
            let mut body = vec![line.into_any_element()];
            body.push(match &command_error {
                Some(_) => error(Field::Command, "command").unwrap(),
                None => self.command_status(kind),
            });
            body.extend(notes);
            body.extend(saved(Field::Command));
            field(first, "Command", hint, body)
        };

        if !installed {
            let hint = kind.install_command();
            fields.push(field(
                true,
                "Install",
                None,
                vec![
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .min_h(px(28.))
                                .flex()
                                .items_center()
                                .rounded_md()
                                .border_1()
                                .border_color(content(0.1))
                                .bg(content(0.03))
                                .px_2()
                                .py_1()
                                .font_family(crate::fonts::mono())
                                .text_size(px(12.))
                                .line_height(px(16.))
                                .text_color(content(0.8))
                                .child(hint.unwrap_or("See the install guide for this agent.")),
                        )
                        .when_some(hint, |d, command| {
                            d.child(
                                secondary_button(
                                    SharedString::from(format!("copy-install-{}", kind.id())),
                                    "Copy the install command",
                                    false,
                                    false,
                                )
                                .child(icon("copy", 13.))
                                .child("Copy")
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            command.to_owned(),
                                        ));
                                        this.agent_toast(
                                            "copy",
                                            "Install command copied.".into(),
                                            None,
                                            cx,
                                        );
                                    },
                                )),
                            )
                        })
                        .child(
                            secondary_button(
                                SharedString::from(format!("install-guide-{}", kind.id())),
                                format!("Open the {} install guide", kind.label()),
                                false,
                                false,
                            )
                            .border_color(accent(0.4))
                            .text_color(accent_color())
                            .child(icon("download", 13.).text_color(accent_color()))
                            .child("Install")
                            .on_click(move |_, _, cx| cx.open_url(kind.docs_url())),
                        )
                        .into_any_element(),
                    help("Install opens this agent's install guide. Refresh when it is installed.")
                        .into_any_element(),
                ],
            ));
            fields.push(command_field(
                false,
                Some("Path override".into()),
                vec![
                    help("Already installed under another name or outside your PATH? Enter the command or its full path.")
                        .into_any_element(),
                ],
                cx,
            ));
            return self.panel_frame(kind, fields);
        }

        // Opens in.
        if !self.ctx.no_terminal_sessions {
            let per_agent = self.prefs.open_mode() == OPEN_PER_AGENT;
            let mut body = vec![self.radio_cards(
                format!("agent-opens-{}", kind.id()),
                format!("{} opens in", kind.label()),
                agent.surface.id(),
                vec![
                    RadioCard {
                        value: Surface::Chat.id(),
                        title: "Chat".into(),
                        description: "Chat in Monoptah".into(),
                        glyph: Some("message-square"),
                        preview: None,
                    },
                    RadioCard {
                        value: Surface::Tui.id(),
                        title: "Terminal (TUI)".into(),
                        description: format!("{}'s own terminal UI", kind.label()).into(),
                        glyph: Some("terminal"),
                        preview: None,
                    },
                ],
                true,
                cx,
                move |this, value, cx| {
                    if let Some(surface) = Surface::parse(value) {
                        this.pick_agent_surface(kind, surface, cx)
                    }
                },
            )];
            if !per_agent {
                body.push(
                    help("Picking a different option here switches \"Open new sessions in\" to Choose per agent.")
                        .into_any_element(),
                );
            }
            body.extend(saved(Field::Surface));
            fields.push(field(
                true,
                "Opens in",
                Some(if per_agent {
                    "Set for this agent".into()
                } else {
                    "Same for all agents".into()
                }),
                body,
            ));
        }

        // Permissions.
        if !Permission::supported(kind).is_empty() {
            let mut body = vec![self.permission_trigger(
                MenuTarget::Permission(kind),
                agent.permission,
                (!agent.permission_overridden).then_some(agent.permission),
                cx,
            )];
            if agent.permission == Permission::Full {
                body.push(
                    inline_banner(
                        SharedString::from(format!("agent-full-warning-{}", kind.id())),
                        "shield-off",
                        permission_color(Permission::Full),
                        permission_color(Permission::Full),
                        format!(
                            "{} will edit files and run commands without asking. Use it only in folders you trust.",
                            kind.label()
                        ),
                        None,
                    )
                    .debug_selector(move || format!("settings-agent-full-warning-{}", kind.id()))
                    .into_any_element(),
                );
            }
            body.extend(saved(Field::Permission));
            fields.push(field(
                fields.is_empty(),
                "Permissions",
                agent
                    .permission_overridden
                    .then(|| "Set for this agent".into()),
                body,
            ));
        }

        // Command. Antigravity's is the ACP server its chat sessions use; its terminal
        // always runs `agy`, so the field says it applies to chat only.
        let (hint, notes) = if kind == Kind::Antigravity {
            (
                Some("Chat sessions".into()),
                vec![
                    help("Terminal sessions always run agy from your PATH.")
                        .debug_selector(|| "settings-agent-command-chat-only".into())
                        .into_any_element(),
                ],
            )
        } else {
            (None, Vec::new())
        };
        fields.push(command_field(fields.is_empty(), hint, notes, cx));

        // Model.
        {
            let catalog = self
                .ctx
                .catalogs
                .get(kind.id())
                .cloned()
                .unwrap_or_default();
            let stored = agent.model.clone();
            let listed = stored
                .as_ref()
                .is_some_and(|model| catalog.iter().any(|(id, _)| id == model));
            let typing = self.agents.custom_model == Some(kind);
            let mut options: Vec<(String, SharedString, Option<AnyElement>)> =
                vec![(String::new(), "Agent default".into(), None)];
            options.extend(
                catalog
                    .iter()
                    .map(|(id, name)| (id.clone(), SharedString::from(name.clone()), None)),
            );
            if let Some(model) = stored.as_ref().filter(|_| !listed && !typing) {
                options.push((model.clone(), format!("{model} · Custom").into(), None));
            }
            options.push((CUSTOM_MODEL.into(), "Custom model…".into(), None));
            let value = if typing {
                CUSTOM_MODEL.to_owned()
            } else {
                stored.clone().unwrap_or_default()
            };
            let mut line = div().flex().items_center().gap_2().child(self.select(
                model_select_id(kind),
                "Model",
                &value,
                options,
                Some(240.),
                cx,
                move |this, value, cx| this.pick_agent_model(kind, value, cx),
            ));
            if typing {
                line = line.child(
                    text_box(inputs.model.clone(), true, false)
                        .debug_selector(move || format!("settings-agent-model-{}", kind.id())),
                );
            }
            let mut body = vec![line.into_any_element()];
            body.push(
                help(match (&stored, listed) {
                    (None, _) => "Agent default passes no model.".to_owned(),
                    (Some(model), false) => format!("Custom · passed as --model {model}"),
                    (Some(_), true) => format!(
                        "{} {} from the catalog.",
                        catalog.len(),
                        if catalog.len() == 1 {
                            "model"
                        } else {
                            "models"
                        }
                    ),
                })
                .into_any_element(),
            );
            body.extend(saved(Field::Model));
            fields.push(field(false, "Model", None, body));
        }

        // Reasoning effort.
        let ladder = launch_args::effort_levels(kind);
        if !ladder.is_empty() {
            let mut levels: Vec<(String, SharedString)> = ladder
                .iter()
                .map(|level| {
                    (
                        (*level).to_owned(),
                        crate::conversation::effort_name(level).into(),
                    )
                })
                .collect();
            // An effort saved before this page (or typed as a flag) keeps its own step.
            if let Some(effort) = agent
                .effort
                .as_ref()
                .filter(|effort| !ladder.contains(&effort.as_str()))
            {
                levels.push((
                    effort.clone(),
                    crate::conversation::effort_name(effort).into(),
                ));
            }
            let mut body = vec![
                div()
                    .flex()
                    .child(self.level_picker(
                        format!("agent-effort-{}", kind.id()),
                        "Reasoning effort",
                        agent.effort.as_deref(),
                        &levels,
                        cx,
                        move |this, level, cx| {
                            this.update(cx, |p| match &level {
                                Some(level) => {
                                    p.agents.efforts.insert(kind.id().into(), level.clone());
                                }
                                None => {
                                    p.agents.efforts.remove(kind.id());
                                }
                            });
                            this.flash_saved(kind, Field::Effort, cx);
                        },
                    ))
                    .into_any_element(),
            ];
            body.extend(saved(Field::Effort));
            fields.push(field(false, "Reasoning effort", None, body));
        }

        // Arguments.
        {
            let args_error = error(Field::Args, "args");
            let draft = inputs.args.read(cx).content.to_string();
            let mut line = div().flex().items_center().gap_2().child(
                text_box(inputs.args.clone(), true, args_error.is_some())
                    .debug_selector(move || format!("settings-agent-args-{}", kind.id())),
            );
            if !agent.args.is_empty() {
                line = line.child(
                    secondary_button(
                        SharedString::from(format!("reset-args-{}", kind.id())),
                        "Clear the arguments",
                        false,
                        false,
                    )
                    .border_0()
                    .child("Reset")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.commit_agent_args(kind, String::new(), cx)
                    })),
                );
            }
            let mut body = vec![line.into_any_element()];
            match args_error {
                Some(error) => body.push(error),
                None => {
                    let parts = launch_args::split(&draft).unwrap_or_default();
                    if !parts.is_empty() {
                        body.push(
                            div()
                                .id(SharedString::from(format!("agent-chips-{}", kind.id())))
                                .debug_selector(move || {
                                    format!("settings-agent-chips-{}", kind.id())
                                })
                                .role(Role::Group)
                                .aria_label("Split arguments")
                                .flex()
                                .flex_wrap()
                                .gap_1()
                                .children(parts.into_iter().map(chip))
                                .into_any_element(),
                        );
                    }
                }
            }
            body.push(
                help("Used by chat and terminal sessions. Permission, model and effort flags move to their own fields. Arguments split with POSIX shell rules, so quote a Windows path or write it with forward slashes.")
                    .into_any_element(),
            );
            body.extend(saved(Field::Args));
            fields.push(field(
                false,
                "Arguments",
                Some("Added after the command".into()),
                body,
            ));
        }

        // Environment.
        {
            let env_error = error(Field::Env, "env");
            let editing = self.agents.editing_env == Some(kind);
            let count = agent.env.len();
            let mut body: Vec<AnyElement> = Vec::new();
            if editing {
                body.push(
                    text_area(inputs.env.clone(), true, env_error.is_some())
                        .debug_selector(move || format!("settings-agent-env-{}", kind.id()))
                        .into_any_element(),
                );
                match env_error {
                    Some(error) => body.push(error),
                    None => body.push(
                        help("One KEY=VALUE per line. Saved when you leave the field.")
                            .into_any_element(),
                    ),
                }
                body.push(
                    div()
                        .flex()
                        .justify_end()
                        .child(
                            secondary_button(
                                SharedString::from(format!("done-env-{}", kind.id())),
                                "Save the variables",
                                false,
                                false,
                            )
                            .debug_selector(move || {
                                format!("settings-agent-env-done-{}", kind.id())
                            })
                            .child("Done")
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.commit_agent_field(kind, Field::Env, cx),
                            )),
                        )
                        .into_any_element(),
                );
            } else if count > 0 {
                let owned = crate::accounts::get(cx)
                    .map(|center| {
                        center
                            .read(cx)
                            .selected(kind, &self.ctx.cwd)
                            .owned_keys(kind)
                    })
                    .unwrap_or_default();
                let rows = agent
                    .env
                    .iter()
                    .enumerate()
                    .map(|(index, (key, value))| {
                        let secret = launch_args::is_secret(key);
                        let shown = !secret || self.agents.revealed.contains(&(kind, key.clone()));
                        let reveal = secret.then(|| {
                            let key = key.clone();
                            icon_action(
                                SharedString::from(format!("reveal-env-{}-{index}", kind.id())),
                                format!("{} {key}", if shown { "Hide" } else { "Reveal" }),
                                if shown { "eye-off" } else { "eye" },
                                24.,
                                14.,
                                0.6,
                                false,
                            )
                            .debug_selector(move || {
                                format!("settings-agent-env-reveal-{}-{index}", kind.id())
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let entry = (kind, key.clone());
                                if !this.agents.revealed.remove(&entry) {
                                    this.agents.revealed.insert(entry);
                                }
                                cx.notify();
                            }))
                            .into_any_element()
                        });
                        env_row(
                            index == 0,
                            key.clone(),
                            if shown {
                                value.clone()
                            } else {
                                "••••••••••••".into()
                            },
                            owned
                                .iter()
                                .any(|name| name == key)
                                .then(|| "Set by the current account".into()),
                            reveal,
                        )
                    })
                    .collect();
                body.push(
                    env_table(rows)
                        .debug_selector(move || format!("settings-agent-env-table-{}", kind.id()))
                        .into_any_element(),
                );
                body.push(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            help("Values whose names contain KEY, TOKEN, SECRET, PASSWORD or AUTH stay hidden.")
                                .min_w_0()
                                .flex_1(),
                        )
                        .child(self.edit_env_button(kind, "pencil", "Edit as text", cx))
                        .into_any_element(),
                );
            } else {
                body.push(
                    div()
                        .flex()
                        .child(self.edit_env_button(kind, "plus", "Add variables", cx))
                        .into_any_element(),
                );
            }
            body.extend(saved(Field::Env));
            fields.push(field(
                false,
                "Environment",
                (count > 0).then(|| {
                    format!(
                        "{count} {}",
                        if count == 1 { "variable" } else { "variables" }
                    )
                    .into()
                }),
                body,
            ));
        }

        // Launch preview.
        let terminal = self.launches_in_terminal(kind);
        let (lead, permission, rest) = self.agent_launch_args(kind);
        let token = |text: String, color| div().text_color(color).child(text);
        let mut tokens: Vec<gpui::Div> = agent
            .env
            .iter()
            .map(|(key, value)| {
                let hidden = launch_args::is_secret(key)
                    && !self.agents.revealed.contains(&(kind, key.clone()));
                token(
                    format!(
                        "{key}={}",
                        if hidden {
                            "••••"
                        } else {
                            value.as_str()
                        }
                    ),
                    content(0.5),
                )
            })
            .collect();
        tokens.push(token(
            command_name(kind, agent.command.as_deref(), terminal),
            content(0.8),
        ));
        tokens.extend(lead.into_iter().map(|flag| token(flag, content(0.8))));
        tokens.extend(
            permission
                .into_iter()
                .map(|flag| token(flag, tinted(permission_color(agent.permission), 1.))),
        );
        tokens.extend(rest.into_iter().map(|arg| token(arg, content(0.8))));
        let mut preview = vec![
            code_block(
                "terminal",
                format!(
                    "Launch preview · {} session",
                    if terminal { "terminal" } else { "chat" }
                ),
                secondary_button(
                    SharedString::from(format!("copy-launch-{}", kind.id())),
                    "Copy the launch command",
                    false,
                    false,
                )
                .debug_selector(move || format!("settings-agent-copy-launch-{}", kind.id()))
                .border_0()
                .child(icon("copy", 12.))
                .child("Copy")
                .on_click(cx.listener(move |this, _, _, cx| this.copy_agent_launch(kind, cx)))
                .into_any_element(),
                div()
                    .debug_selector(move || format!("settings-agent-preview-{}", kind.id()))
                    .flex()
                    .flex_wrap()
                    .gap_x(px(7.))
                    .children(tokens)
                    .into_any_element(),
            )
            .mt_2()
            .into_any_element(),
        ];
        if terminal && !monocode_core::surface::tui_caps(kind).launch_flags {
            preview.push(
                help(format!(
                    "{}'s terminal takes only the arguments and environment. Its model, effort and permissions apply to chat sessions.",
                    kind.label()
                ))
                .mt(px(6.))
                .into_any_element(),
            );
        } else if !terminal && launch_args::chat_uses_protocol(kind) {
            preview.push(
                help("Chat sessions send the model, effort and permissions over the agent's protocol, not as flags.")
                    .mt(px(6.))
                    .into_any_element(),
            );
        }
        fields.extend(preview);
        self.panel_frame(kind, fields)
    }

    /// The panel under an open row's header. It has no fill, rule or hover of its own: the
    /// row's open fill runs behind both.
    fn panel_frame(&self, kind: Kind, fields: Vec<AnyElement>) -> AnyElement {
        div()
            .debug_selector(move || format!("settings-agent-panel-{}", kind.id()))
            .pl(px(52.))
            .pr_4()
            .pt_1()
            .pb_4()
            .children(fields)
            .into_any_element()
    }

    fn edit_env_button(
        &self,
        kind: Kind,
        glyph: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        secondary_button(
            SharedString::from(format!("edit-env-{}", kind.id())),
            label,
            false,
            false,
        )
        .debug_selector(move || format!("settings-agent-env-edit-{}", kind.id()))
        .child(icon(glyph, 12.))
        .child(label)
        .on_click(cx.listener(move |this, _, window, cx| {
            this.agents.editing_env = Some(kind);
            this.agents.errors.remove(&(kind, Field::Env));
            if let Some(inputs) = this.agents.inputs.get(&kind) {
                let text = this
                    .prefs
                    .agents
                    .env
                    .get(kind.id())
                    .cloned()
                    .unwrap_or_default();
                inputs.env.update(cx, |input, cx| input.set_text(text, cx));
                window.focus(&inputs.env.focus_handle(cx), cx);
            }
            cx.notify();
        }))
        .into_any_element()
    }

    /// The line under the Command field: where the CLI is and which version answered.
    fn command_status(&self, kind: Kind) -> AnyElement {
        let configured = self.prefs.agent(kind).command.is_some();
        if self.agents.inspecting.contains_key(&kind) {
            return status_line(content(0.3), vec![("Checking the CLI…".into(), false)]);
        }
        match self.agents.inspections.get(&kind) {
            Some(Ok(inspection)) => status_line(
                if configured {
                    warning_color()
                } else {
                    success_color()
                },
                vec![
                    (
                        if configured {
                            "Configured"
                        } else {
                            "Auto-detected"
                        }
                        .into(),
                        false,
                    ),
                    (inspection.path.display().to_string().into(), true),
                    (inspection.version.clone().into(), true),
                ],
            ),
            Some(Err(error)) => status_line(danger_color(), vec![(error.clone().into(), false)]),
            None => match self.agent_path(kind) {
                Some(path) => status_line(
                    if configured {
                        warning_color()
                    } else {
                        success_color()
                    },
                    vec![
                        (
                            if configured {
                                "Configured"
                            } else {
                                "Auto-detected"
                            }
                            .into(),
                            false,
                        ),
                        (path.display().to_string().into(), true),
                    ],
                ),
                None => status_line(content(0.3), vec![("Not found on your PATH".into(), false)]),
            },
        }
    }

    // --- State changes.

    /// Open one row and close the others. Text typed in the row being left is saved first,
    /// as leaving its field would.
    pub(in crate::settings) fn set_expanded_agent(
        &mut self,
        kind: Option<Kind>,
        cx: &mut Context<Self>,
    ) {
        if self.agents.expanded == kind {
            self.agents.menu = None;
            return;
        }
        if let Some(open) = self.agents.expanded {
            for field in [Field::Command, Field::Model, Field::Args, Field::Env] {
                self.commit_agent_field(open, field, cx);
            }
            // Text that did not validate goes with the row: its error must not sit under
            // the saved value when the row opens again.
            self.agents.errors.retain(|(agent, _), _| *agent != open);
        }
        // The row being left closes while the new one opens, both over the same time.
        let (animate, now) = (!cx.reduce_motion(), Instant::now());
        if let Some(closing) = self.agents.expanded
            && let Some(disclosure) = self.agents.disclosures.get_mut(&closing)
        {
            disclosure.set(false, animate, now);
        }
        if let Some(opening) = kind {
            self.agents
                .disclosures
                .entry(opening)
                .or_insert_with(|| Disclosure::new(false))
                .set(true, animate, now);
        }
        self.agents.expanded = kind;
        self.agents.menu = None;
        self.agents.editing_env = None;
        self.agents.custom_model = None;
        self.select_open = None;
        // Only an installed agent's CLI is checked on its own: checking starts it.
        if let Some(kind) = kind
            && self.agent_path(kind).is_some()
            && !self.agents.inspections.contains_key(&kind)
        {
            self.inspect_agent(kind, None, false, false, cx);
        }
        cx.notify();
    }

    /// Create the expanded row's text fields, or bring them in step with the saved values
    /// while nobody is typing in them.
    fn ensure_agent_inputs(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        let agent = self.prefs.agent(kind);
        let args = self
            .prefs
            .agents
            .args
            .get(kind.id())
            .cloned()
            .unwrap_or_default();
        let env = self
            .prefs
            .agents
            .env
            .get(kind.id())
            .cloned()
            .unwrap_or_default();
        let command = agent.command.clone().unwrap_or_default();
        let model = agent.model.clone().unwrap_or_default();
        if let Some(inputs) = self.agents.inputs.get_mut(&kind) {
            let editing_env = self.agents.editing_env == Some(kind);
            let typing_model = self.agents.custom_model == Some(kind);
            for (input, field, stored, skip) in [
                (inputs.command.clone(), Field::Command, command, false),
                (inputs.model.clone(), Field::Model, model, typing_model),
                (inputs.args.clone(), Field::Args, args, false),
                (inputs.env.clone(), Field::Env, env, editing_env),
            ] {
                if skip || inputs.seen.get(&field) == Some(&stored) {
                    continue;
                }
                let in_step = *input.read(cx).content == *stored;
                // A field being typed in keeps its text. One that already shows the new
                // value (Enter saved it) is in step even while focused: left stale, text
                // typed after that Enter would be put back to the saved value on blur.
                if !in_step && input.focus_handle(cx).is_focused(window) {
                    continue;
                }
                inputs.seen.insert(field, stored.clone());
                if !in_step {
                    input.update(cx, |input, cx| input.set_text(stored, cx));
                }
            }
            return;
        }
        let seen = HashMap::from([
            (Field::Command, command.clone()),
            (Field::Model, model.clone()),
            (Field::Args, args.clone()),
            (Field::Env, env.clone()),
        ]);
        let text_field =
            |label: String, placeholder: String, text: String, cx: &mut Context<Self>| {
                cx.new(|cx| {
                    let mut input = Input::field(label, placeholder, content(0.35), cx);
                    input.set_font_size(12.);
                    input.set_text(text, cx);
                    input
                })
            };
        let command = text_field(
            format!("{} command", kind.label()),
            command_name(kind, None, false),
            command,
            cx,
        );
        let model = text_field(
            format!("{} custom model", kind.label()),
            "model name".into(),
            model,
            cx,
        );
        let args = text_field(
            format!("{} arguments", kind.label()),
            "--add-dir ../shared".into(),
            args,
            cx,
        );
        let label = format!("{} environment", kind.label());
        let env = cx.new(|cx| {
            let mut input = Input::multiline_field(&label, "KEY=VALUE, one per line", cx);
            input.set_text(env, cx);
            input
        });
        let mut subscriptions = Vec::new();
        for (input, field) in [
            (&command, Field::Command),
            (&model, Field::Model),
            (&args, Field::Args),
            (&env, Field::Env),
        ] {
            let focus = input.focus_handle(cx);
            subscriptions.push(cx.on_blur(&focus, window, move |this, _, cx| {
                this.commit_agent_field(kind, field, cx)
            }));
            subscriptions.push(cx.subscribe(input, |_, _, _: &input::Changed, cx| cx.notify()));
            if field != Field::Env {
                subscriptions.push(cx.subscribe(
                    input,
                    move |this, _, _: &input::Submitted, cx| {
                        this.commit_agent_field(kind, field, cx)
                    },
                ));
            }
        }
        self.agents.inputs.insert(
            kind,
            Inputs {
                command,
                model,
                args,
                env,
                seen,
                _subscriptions: subscriptions,
            },
        );
    }

    /// Save a text field, as leaving it or pressing Enter does. A value that does not
    /// validate marks the field and leaves the saved one in effect.
    fn commit_agent_field(&mut self, kind: Kind, field: Field, cx: &mut Context<Self>) {
        let Some(inputs) = self.agents.inputs.get(&kind) else {
            return;
        };
        let input = match field {
            Field::Command => &inputs.command,
            Field::Model => &inputs.model,
            Field::Args => &inputs.args,
            Field::Env => &inputs.env,
            _ => return,
        };
        let draft = input.read(cx).content.to_string();
        match field {
            Field::Command => self.commit_agent_command(kind, draft.trim().to_owned(), cx),
            Field::Args => self.commit_agent_args(kind, draft, cx),
            Field::Env if self.agents.editing_env == Some(kind) => {
                self.commit_agent_env(kind, draft, cx)
            }
            Field::Model if self.agents.custom_model == Some(kind) => {
                self.commit_agent_model(kind, draft.trim().to_owned(), cx)
            }
            _ => {}
        }
    }

    /// "Check version" beside the Command field. A command typed but not yet saved is the
    /// one checked (and saved when it passes): checking the saved one instead would also
    /// supersede, and so drop, the check that leaving the field has just started.
    fn check_agent_command(&mut self, kind: Kind, cx: &mut Context<Self>) {
        let draft = self
            .agents
            .inputs
            .get(&kind)
            .map(|inputs| inputs.command.read(cx).content.trim().to_owned());
        let stored = self.prefs.agent(kind).command.unwrap_or_default();
        match draft {
            Some(draft) if draft != stored => self.commit_agent_command(kind, draft, cx),
            _ => self.inspect_agent(kind, None, false, false, cx),
        }
    }

    /// The command is checked before it is saved: one that does not resolve, or whose
    /// `--version` fails, never replaces a working one. An empty path goes back to the
    /// auto-detected CLI.
    pub(in crate::settings) fn commit_agent_command(
        &mut self,
        kind: Kind,
        draft: String,
        cx: &mut Context<Self>,
    ) {
        let key = (kind, Field::Command);
        let stored = self.prefs.agent(kind).command.unwrap_or_default();
        if draft == stored {
            // The saved command is the latest choice again: a check of another one still
            // under way must not replace it when it passes.
            self.cancel_command_save(kind);
            if self.agents.errors.remove(&key).is_some() {
                cx.notify();
            }
            return;
        }
        if draft.is_empty() {
            self.cancel_command_save(kind);
            self.agents.errors.remove(&key);
            self.update(cx, |p| {
                p.binary_paths.remove(kind.id());
            });
            if let Some(inputs) = self.agents.inputs.get(&kind) {
                inputs
                    .command
                    .update(cx, |input, cx| input.set_text(String::new(), cx));
            }
            self.flash_saved(kind, Field::Command, cx);
            self.inspect_agent(kind, None, false, false, cx);
            return;
        }
        self.inspect_agent(kind, Some(draft), true, false, cx);
    }

    /// Supersede the check of a command being saved, so its result saves nothing.
    fn cancel_command_save(&mut self, kind: Kind) {
        *self.agents.save_generation.entry(kind).or_default() += 1;
    }

    fn commit_agent_args(&mut self, kind: Kind, draft: String, cx: &mut Context<Self>) {
        let key = (kind, Field::Args);
        let id = kind.id();
        let stored = self.prefs.agents.args.get(id).cloned().unwrap_or_default();
        if draft.trim() == stored.trim() {
            if self.agents.errors.remove(&key).is_some() {
                cx.notify();
            }
            return;
        }
        let parts = match launch_args::split(&draft) {
            Ok(parts) => parts,
            Err(error) => {
                self.agents.errors.insert(
                    key,
                    format!("{error} The previous arguments stay in effect."),
                );
                cx.notify();
                return;
            }
        };
        // A terminal that passes no flags of its own would lose a moved flag, so its typed
        // flags stay in the arguments (chat launches drop them: `chat_user_args`).
        let extracted =
            if self.launches_in_terminal(kind) && launch_args::terminal_keeps_typed_flags(kind) {
                launch_args::Extracted {
                    rest: parts.clone(),
                    ..Default::default()
                }
            } else {
                launch_args::extract_structured(kind, &parts)
            };
        // `extracted.moved` is text for the notice; Undo needs the arguments themselves,
        // so that one with a space in it goes back as one argument.
        let moved_args = moved_tokens(&parts, &extracted.rest);
        let permission = extracted
            .permission
            .map(Permission::from_access)
            .filter(|level| Permission::supported(kind).contains(level));
        // Typed text stays as typed unless a flag left it.
        let line = if extracted.moved.is_empty() {
            draft.trim().to_owned()
        } else {
            launch_args::join(&extracted.rest)
        };
        // What Undo restores: the fields the moved flags wrote, as they were.
        let model_before = extracted
            .model
            .is_some()
            .then(|| self.prefs.default_models.get(id).cloned());
        let effort_before = extracted
            .effort
            .is_some()
            .then(|| self.prefs.agents.efforts.get(id).cloned());
        let permission_before = permission.map(|_| self.prefs.agents.permissions.get(id).cloned());
        self.agents.errors.remove(&key);
        self.update(cx, |p| {
            if line.is_empty() {
                p.agents.args.remove(id);
            } else {
                p.agents.args.insert(id.into(), line.clone());
            }
            if let Some(model) = &extracted.model {
                p.default_models.insert(id.into(), model.clone());
            }
            if let Some(effort) = &extracted.effort {
                p.agents.efforts.insert(id.into(), effort.clone());
            }
            if let Some(level) = permission {
                p.agents.permissions.insert(id.into(), level.id().into());
            }
        });
        if let Some(inputs) = self.agents.inputs.get(&kind) {
            inputs
                .args
                .update(cx, |input, cx| input.set_text(line.clone(), cx));
        }
        if extracted.moved.is_empty() {
            self.flash_saved(kind, Field::Args, cx);
        } else {
            self.agent_toast(
                "arrow-right-left",
                format!(
                    "Moved {} out of Arguments into {}.",
                    extracted.moved.join(", "),
                    if extracted.moved.len() == 1 {
                        "its own field"
                    } else {
                        "their own fields"
                    }
                ),
                // Undo puts the moved flags back at the end of the arguments as they are
                // now and restores the fields they wrote. Everything else typed in the
                // same edit, or since, stays.
                Some(Box::new({
                    let moved = launch_args::join(&moved_args);
                    move |p: &mut Preferences| {
                        let line = p.agents.args.get(id).cloned().unwrap_or_default();
                        let line = [line.trim(), moved.as_str()]
                            .into_iter()
                            .filter(|part| !part.is_empty())
                            .collect::<Vec<_>>()
                            .join(" ");
                        p.agents.args.insert(id.into(), line);
                        if let Some(before) = model_before {
                            restore(&mut p.default_models, id, before);
                        }
                        if let Some(before) = effort_before {
                            restore(&mut p.agents.efforts, id, before);
                        }
                        if let Some(before) = permission_before {
                            restore(&mut p.agents.permissions, id, before);
                        }
                    }
                })),
                cx,
            );
        }
    }

    fn commit_agent_env(&mut self, kind: Kind, draft: String, cx: &mut Context<Self>) {
        let key = (kind, Field::Env);
        let id = kind.id();
        if let Err(error) = launch_args::parse_env(&draft) {
            self.agents.errors.insert(
                key,
                format!("{error} The previous variables stay in effect."),
            );
            cx.notify();
            return;
        }
        let text = draft
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let stored = self.prefs.agents.env.get(id).cloned().unwrap_or_default();
        self.agents.errors.remove(&key);
        self.agents.editing_env = None;
        if text != stored {
            self.update(cx, |p| {
                if text.is_empty() {
                    p.agents.env.remove(id);
                } else {
                    p.agents.env.insert(id.into(), text.clone());
                }
            });
            self.flash_saved(kind, Field::Env, cx);
        }
        cx.notify();
    }

    /// A typed model name is passed as it is.
    fn commit_agent_model(&mut self, kind: Kind, draft: String, cx: &mut Context<Self>) {
        self.agents.custom_model = None;
        if !draft.is_empty() && self.prefs.agent(kind).model.as_deref() != Some(draft.as_str()) {
            self.update(cx, |p| {
                p.default_models.insert(kind.id().into(), draft);
            });
            self.flash_saved(kind, Field::Model, cx);
        }
        cx.notify();
    }

    fn pick_agent_model(&mut self, kind: Kind, value: String, cx: &mut Context<Self>) {
        if value == CUSTOM_MODEL {
            self.agents.custom_model = Some(kind);
            cx.notify();
            return;
        }
        self.agents.custom_model = None;
        self.update(cx, |p| {
            if value.is_empty() {
                p.default_models.remove(kind.id());
            } else {
                p.default_models.insert(kind.id().into(), value);
            }
        });
        self.flash_saved(kind, Field::Model, cx);
    }

    /// "Opens in" inside a row. Picking the other surface while every agent shares one
    /// switches to per agent, which the notice says and can undo.
    fn pick_agent_surface(&mut self, kind: Kind, surface: Surface, cx: &mut Context<Self>) {
        let current = self.prefs.agent(kind).surface;
        if surface == current {
            return;
        }
        if self.prefs.open_mode() == OPEN_PER_AGENT {
            self.update(cx, |p| p.set_agent_surface(kind, surface));
            self.flash_saved(kind, Field::Surface, cx);
            return;
        }
        let mode = self.prefs.agents.open_mode.clone();
        let surfaces = self.prefs.provider_surfaces.clone();
        self.update(cx, |p| p.set_agent_surface(kind, surface));
        self.agent_toast(
            "arrow-right-left",
            format!(
                "Open mode is now set per agent. {} opens in {}; the others keep {}.",
                kind.label(),
                surface_name(surface),
                surface_name(current)
            ),
            Some(Box::new(move |p: &mut Preferences| {
                p.agents.open_mode = mode;
                p.provider_surfaces = surfaces;
            })),
            cx,
        );
    }

    /// The row's switch. Turning off the default agent clears the default, which the
    /// notice says and can undo.
    fn set_agent_enabled(&mut self, kind: Kind, enabled: bool, cx: &mut Context<Self>) {
        let hidden = self.prefs.hidden_providers.clone();
        let default = self.prefs.default_provider.clone();
        let mut cleared = false;
        self.update(cx, |p| cleared = p.set_agent_enabled(kind, enabled));
        if cleared {
            self.agent_toast(
                "circle-alert",
                format!(
                    "{} is off, so it is no longer the default. New sessions use your most recent agent.",
                    kind.label()
                ),
                Some(Box::new(move |p: &mut Preferences| {
                    p.hidden_providers = hidden;
                    p.default_provider = default;
                })),
                cx,
            );
        }
    }

    fn set_agent_permission(
        &mut self,
        target: MenuTarget,
        level: Option<Permission>,
        cx: &mut Context<Self>,
    ) {
        match target {
            MenuTarget::GlobalPermission => {
                let Some(level) = level else { return };
                if self.prefs.agent_permission() == level {
                    return;
                }
                let before = self.prefs.agents.permission.clone();
                self.update(cx, |p| p.agents.permission = Some(level.id().into()));
                if level == Permission::Full {
                    self.agent_toast(
                        "shield-off",
                        "Agents now have full access. They won't ask before editing files or running commands."
                            .into(),
                        Some(Box::new(move |p: &mut Preferences| p.agents.permission = before)),
                        cx,
                    );
                }
            }
            MenuTarget::Permission(kind) => {
                let id = kind.id();
                let before = self.prefs.agents.permissions.get(id).cloned();
                self.update(cx, |p| match level {
                    Some(level) => {
                        p.agents
                            .permissions
                            .insert(kind.id().into(), level.id().into());
                    }
                    None => {
                        p.agents.permissions.remove(kind.id());
                    }
                });
                if before == self.prefs.agents.permissions.get(id).cloned() {
                    return;
                }
                self.flash_saved(kind, Field::Permission, cx);
                if level == Some(Permission::Full) {
                    self.agent_toast(
                        "shield-off",
                        format!(
                            "{} now has full access. It won't ask before editing files or running commands.",
                            kind.label()
                        ),
                        Some(Box::new(move |p: &mut Preferences| {
                            restore(&mut p.agents.permissions, id, before)
                        })),
                        cx,
                    );
                }
            }
            MenuTarget::Row(_) => {}
        }
    }

    /// Put an agent back to how a new install has it: auto-detected CLI, its own model and
    /// effort, the shared permission, no arguments and no variables.
    fn reset_agent(&mut self, kind: Kind, cx: &mut Context<Self>) {
        let id = kind.id();
        let before = [
            self.prefs.binary_paths.get(id).cloned(),
            self.prefs.default_models.get(id).cloned(),
            self.prefs.agents.permissions.get(id).cloned(),
            self.prefs.agents.efforts.get(id).cloned(),
            self.prefs.agents.args.get(id).cloned(),
            self.prefs.agents.env.get(id).cloned(),
        ];
        if before.iter().all(Option::is_none) {
            return;
        }
        self.update(cx, |p| {
            p.binary_paths.remove(id);
            p.default_models.remove(id);
            p.agents.permissions.remove(id);
            p.agents.efforts.remove(id);
            p.agents.args.remove(id);
            p.agents.env.remove(id);
        });
        self.cancel_command_save(kind);
        self.agents.errors.retain(|(agent, _), _| *agent != kind);
        self.agents.revealed.retain(|(agent, _)| *agent != kind);
        self.agents.inspections.remove(&kind);
        if self.agents.editing_env == Some(kind) {
            self.agents.editing_env = None;
        }
        self.agent_toast(
            "rotate-ccw",
            format!("{} settings reset.", kind.label()),
            Some(Box::new(move |p: &mut Preferences| {
                let [command, model, permission, effort, args, env] = before;
                restore(&mut p.binary_paths, id, command);
                restore(&mut p.default_models, id, model);
                restore(&mut p.agents.permissions, id, permission);
                restore(&mut p.agents.efforts, id, effort);
                restore(&mut p.agents.args, id, args);
                restore(&mut p.agents.env, id, env);
            })),
            cx,
        );
    }

    fn copy_agent_launch(&mut self, kind: Kind, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(
            self.agent_launch_line(kind, false),
        ));
        self.agent_toast("copy", "Launch command copied.".into(), None, cx);
    }

    /// Resolve the agent's CLI and ask its version on a worker. With `save`, `draft` is a
    /// path the user typed: it becomes the command only when the check passes. `announce`
    /// reports the result in a notice ("Check version" in the row's menu).
    pub(in crate::settings) fn inspect_agent(
        &mut self,
        kind: Kind,
        draft: Option<String>,
        save: bool,
        announce: bool,
        cx: &mut Context<Self>,
    ) {
        // A save and a background refresh each supersede only their own kind of check.
        let generation = {
            let generations = if save {
                &mut self.agents.save_generation
            } else {
                &mut self.agents.inspect_generation
            };
            let generation = generations.entry(kind).or_default();
            *generation += 1;
            *generation
        };
        *self.agents.inspecting.entry(kind).or_default() += 1;
        let path = draft.clone().or_else(|| self.prefs.agent(kind).command);
        let checked = path.clone();
        let inspector = self.agents.inspector.clone();
        let work = cx.background_executor().spawn(async move {
            match inspector {
                Some(inspect) => inspect(kind, path.as_deref()),
                None => monocode_core::cli::inspect(kind, path.as_deref()),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                // Every check ends here, superseded or not, so "Checking" lasts until the
                // last one has.
                if let Some(count) = this.agents.inspecting.get_mut(&kind) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        this.agents.inspecting.remove(&kind);
                    }
                }
                cx.notify();
                let generations = if save {
                    &this.agents.save_generation
                } else {
                    &this.agents.inspect_generation
                };
                if generations.get(&kind) != Some(&generation) {
                    return;
                }
                // A refresh of a command that has since been replaced describes a CLI that
                // new sessions no longer run.
                if !save && checked != this.prefs.agent(kind).command {
                    return;
                }
                let key = (kind, Field::Command);
                match (&result, save) {
                    (Ok(_), true) => {
                        let value = draft.clone().unwrap_or_default();
                        this.agents.errors.remove(&key);
                        this.update(cx, |p| {
                            p.binary_paths.insert(kind.id().into(), value);
                        });
                        this.agents.inspections.insert(kind, result.clone());
                        this.flash_saved(kind, Field::Command, cx);
                    }
                    // The row may have closed meanwhile, and its typed path with it.
                    (Err(error), true) if this.agents.expanded == Some(kind) => {
                        this.agents
                            .errors
                            .insert(key, format!("{error} The previous path stays in effect."));
                    }
                    (Err(_), true) => {}
                    _ => {
                        this.agents.inspections.insert(kind, result.clone());
                    }
                }
                if announce {
                    match &result {
                        Ok(inspection) => this.agent_toast(
                            "check",
                            format!("{} · {}", kind.label(), inspection.version),
                            None,
                            cx,
                        ),
                        Err(error) => this.agent_toast(
                            "circle-alert",
                            format!("{}: {error}", kind.label()),
                            None,
                            cx,
                        ),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Tie each update offer to the command in effect when it first shows, and forget the
    /// offers the shell no longer has.
    fn bind_update_offers(&mut self) {
        let offered: Vec<Kind> = self
            .ctx
            .agent_updates
            .iter()
            .map(|update| update.kind)
            .collect();
        self.agents
            .offer_commands
            .retain(|kind, _| offered.contains(kind));
        for kind in offered {
            let command = self.prefs.agent(kind).command;
            self.agents.offer_commands.entry(kind).or_insert(command);
        }
    }

    /// The shell's updates changed: an offer that is new or changed comes from a check or
    /// an update that resolved the command in effect now, so it is tied again to that
    /// command when it next shows. An unchanged offer keeps its command.
    pub(in crate::settings) fn agent_updates_changed(
        &mut self,
        updates: &[crate::settings::AgentUpdate],
    ) {
        let before = &self.ctx.agent_updates;
        self.agents.offer_commands.retain(|kind, _| {
            let find = |list: &[crate::settings::AgentUpdate]| {
                list.iter().find(|update| update.kind == *kind).cloned()
            };
            find(before) == find(updates)
        });
    }

    /// The agent's update offer was found for a command other than the one in effect.
    fn update_offer_stale(&self, kind: Kind) -> bool {
        self.agents
            .offer_commands
            .get(&kind)
            .is_some_and(|command| *command != self.prefs.agent(kind).command)
    }

    /// Replace a stale update offer with a check of the CLI in effect: the offer is tied to
    /// the current command, and the shell's new check replaces or confirms it.
    fn recheck_update_offer(&mut self, kind: Kind, cx: &mut Context<Self>) {
        self.agents
            .offer_commands
            .insert(kind, self.prefs.agent(kind).command);
        cx.emit(SettingsEvent::CheckHarnessUpdates);
        cx.notify();
    }

    /// Detection finished: note when, and ask each installed CLI its version for its row.
    pub(in crate::settings) fn agents_detected(&mut self, cx: &mut Context<Self>) {
        self.agents.checked_at = Some(Instant::now());
        self.agents.inspections.clear();
        self.inspect_installed_agents(cx);
    }

    /// Ask each installed CLI its version, once: when detection finishes on the Providers
    /// page, or when the page is opened after it did.
    pub(in crate::settings) fn inspect_installed_agents(&mut self, cx: &mut Context<Self>) {
        if self.section != crate::settings::Section::Providers {
            return;
        }
        let installed: Vec<Kind> = self
            .discovery
            .binaries
            .iter()
            .filter(|(kind, path)| {
                path.is_some()
                    && !self.agents.inspections.contains_key(kind)
                    && !self.agents.inspecting.contains_key(kind)
            })
            .map(|(kind, _)| *kind)
            .collect();
        for kind in installed {
            self.inspect_agent(kind, None, false, false, cx);
        }
    }

    fn flash_saved(&mut self, kind: Kind, field: Field, cx: &mut Context<Self>) {
        self.agents.saved = Some((kind, field));
        self.agents._saved = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVED_FLASH).await;
            let _ = this.update(cx, |this, cx| {
                this.agents.saved = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Show a notice for five seconds; `undo` takes its change back.
    fn agent_toast(
        &mut self,
        glyph: &'static str,
        text: String,
        undo: Option<Undo>,
        cx: &mut Context<Self>,
    ) {
        self.agents.toast = Some(Toast {
            glyph,
            text: text.into(),
            undo,
        });
        self.agents._toast = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_TIME).await;
            let _ = this.update(cx, |this, cx| {
                this.agents.toast = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// The preferences changed some other way. A notice's Undo restores the fields its
    /// change wrote, and a later change may have written the same ones: the notice stays,
    /// its Undo goes.
    pub(in crate::settings) fn agent_undo_expired(&mut self) {
        if let Some(toast) = &mut self.agents.toast {
            toast.undo = None;
        }
    }

    fn undo_agent_toast(&mut self, cx: &mut Context<Self>) {
        let Some(toast) = self.agents.toast.take() else {
            return;
        };
        self.agents._toast = None;
        if let Some(undo) = toast.undo {
            self.agents.errors.clear();
            self.agents.saved = None;
            self.update(cx, undo);
        }
        cx.notify();
    }

    /// The notice, at the bottom right of the Settings view.
    pub(in crate::settings) fn agent_toast_overlay(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let toast = self.agents.toast.as_ref()?;
        let undo = toast.undo.is_some().then(|| {
            secondary_button("settings-toast-undo", "Undo", false, false)
                .debug_selector(|| "settings-toast-undo".into())
                .child("Undo")
                .on_click(cx.listener(|this, _, _, cx| this.undo_agent_toast(cx)))
                .into_any_element()
        });
        Some(
            div()
                .absolute()
                .right(px(24.))
                .bottom(px(24.))
                .child(settings_toast(toast.glyph, toast.text.clone(), undo))
                .into_any_element(),
        )
    }

    // --- Menus.

    fn toggle_agent_menu(
        &mut self,
        target: MenuTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .agents
            .menu
            .as_ref()
            .is_some_and(|open| open.target == target)
        {
            self.agents.menu = None;
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        }
        self.select_open = None;
        let (label, items, width, style) = match target {
            MenuTarget::Row(kind) => (
                format!("{} actions", kind.label()),
                self.agent_actions(kind),
                220.,
                MenuStyle::Explorer,
            ),
            MenuTarget::GlobalPermission => (
                "Permissions for every agent".to_owned(),
                self.permission_items(None),
                340.,
                MenuStyle::Access,
            ),
            MenuTarget::Permission(kind) => (
                format!("{} permissions", kind.label()),
                self.permission_items(Some(kind)),
                340.,
                MenuStyle::Access,
            ),
        };
        let menu = cx.new(|cx| {
            Menu::new(label, items, window, cx)
                .width(width)
                .style(style)
        });
        let events = cx.subscribe(&menu, move |this, _, event: &MenuEvent, cx| {
            this.agents.menu = None;
            if let MenuEvent::Picked(id, _) = event {
                this.agent_menu_picked(target, id, cx);
            }
            cx.notify();
        });
        self.agents.menu = Some(OpenMenu {
            target,
            menu,
            _events: events,
        });
        cx.notify();
    }

    /// The rows of a permission picker: every level for the shared default, or "Same as
    /// all agents" and the levels one agent can apply, each with the flag it passes.
    fn permission_items(&self, agent: Option<Kind>) -> Vec<MenuItem> {
        let level_action = |level: Permission| {
            MenuAction::new(format!("permission:{}", level.id()), level.label())
                .icon(permission_icon(level))
                .icon_color(permission_color(level))
                .description(level.description())
        };
        let Some(kind) = agent else {
            let global = self.prefs.agent_permission();
            let mut items: Vec<MenuItem> = Permission::ALL
                .into_iter()
                .map(|level| level_action(level).checked(level == global).into())
                .collect();
            items.push(MenuItem::Separator);
            items.push(MenuItem::Note(
                "Agents that don't support a mode fall back to their own settings.".into(),
            ));
            return items;
        };
        let config = self.prefs.agent(kind);
        let terminal = self.launches_in_terminal(kind);
        let own = config.permission_overridden.then_some(config.permission);
        let follows = {
            let mut prefs = self.prefs.clone();
            prefs.agents.permissions.remove(kind.id());
            prefs.agent(kind).permission
        };
        let mut items: Vec<MenuItem> = vec![
            MenuAction::new("permission:inherit", "Same as all agents")
                .icon("link")
                .icon_color(permission_color(follows))
                .description(format!(
                    "Follows the default above, now {}.",
                    follows.label()
                ))
                .checked(own.is_none())
                .into(),
            MenuItem::Separator,
        ];
        items.extend(Permission::supported(kind).iter().map(|level| {
            level_action(*level)
                .detail(permission_detail(kind, *level, terminal))
                .checked(own == Some(*level))
                .into()
        }));
        items
    }

    fn agent_actions(&self, kind: Kind) -> Vec<MenuItem> {
        let docs = MenuAction::new("docs", "Docs").icon("book-open");
        if self.agent_path(kind).is_none() {
            return vec![docs.into()];
        }
        let agent = self.prefs.agent(kind);
        let default = if self.prefs.default_provider.as_deref() == Some(kind.id()) {
            MenuAction::new("default:clear", "Clear default").icon("star-off")
        } else {
            MenuAction::new("default:set", "Use by default")
                .icon("star")
                .disabled(!agent.enabled)
        };
        vec![
            default.into(),
            MenuAction::new("check", "Check version")
                .icon("refresh-cw")
                .into(),
            MenuAction::new("reveal", "Open CLI location")
                .icon("folder-open")
                .into(),
            MenuAction::new("copy", "Copy launch command")
                .icon("copy")
                .into(),
            MenuItem::Separator,
            MenuAction::new("reset", "Reset all settings")
                .icon("rotate-ccw")
                .into(),
            MenuItem::Separator,
            docs.into(),
        ]
    }

    fn agent_menu_picked(&mut self, target: MenuTarget, id: &str, cx: &mut Context<Self>) {
        if let Some(level) = id.strip_prefix("permission:") {
            self.set_agent_permission(target, Permission::parse(level), cx);
            return;
        }
        let MenuTarget::Row(kind) = target else {
            return;
        };
        match id {
            "default:set" => self.update(cx, |p| p.default_provider = Some(kind.id().into())),
            "default:clear" => self.update(cx, |p| p.default_provider = None),
            "check" => {
                self.inspect_agent(kind, None, false, true, cx);
                cx.emit(SettingsEvent::CheckHarnessUpdates);
            }
            "reveal" => {
                // The same CLI as the Command field's folder button: the one in effect.
                if let Some(path) = self.active_cli_path(kind) {
                    cx.reveal_path(&path);
                }
            }
            "copy" => self.copy_agent_launch(kind, cx),
            "reset" => self.reset_agent(kind, cx),
            "docs" => cx.open_url(kind.docs_url()),
            _ => {}
        }
    }

    /// The open menu's popover, anchored to its trigger.
    pub(in crate::settings) fn agent_menu_overlay(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.section != crate::settings::Section::Providers {
            return None;
        }
        let open = self.agents.menu.as_ref()?;
        let slot = self.agents.anchors.borrow().get(&open.target).cloned()?;
        Some(
            menu_popover(&open.menu, Anchor::Slot(slot), cx)
                .side(Side::Bottom)
                .align(Align::End)
                .into_any_element(),
        )
    }
}
