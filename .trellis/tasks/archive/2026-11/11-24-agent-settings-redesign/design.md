# Design: Agent CLIs settings redesign

Requirements: `prd.md`. Approved visuals: Ryugi artboard `artboard/agent-clis-settings` (v2).
Evidence: `research/launch-wiring.md`, `research/preferences-migration.md`,
`research/ui-components.md` (all line numbers below come from there; re-check before editing).

## 1. Boundaries

| Layer | Owns | Does not own |
|---|---|---|
| `monocode-core` | Permission levels and their per-agent mapping, argument splitting and flag extraction, env parsing, injecting args/env into chat and TUI launches, reloadable CLI path map | Preferences, UI |
| `monocode-gpui` `settings/preferences.rs` | Stored fields, one-time migration, the effective-config accessor | Launching |
| `monocode-gpui` shell / conversation | Seeding new sessions from the accessor, new-session picker rule | Remembering composer choices |
| `monocode-gpui` `settings/pages/providers.rs` + shared widgets | The page | Business rules (call core / accessor) |

Remote (`remote://`) sessions run through the Host and are out of scope: they keep today's behaviour.

## 2. Data model (extend the existing maps)

Keep these fields with the same JSON keys and meaning, so the React localStorage importer
(`desktop_preferences/workspace_preferences.rs`) needs no change:

| Field | Meaning after the redesign |
|---|---|
| `binary_paths` | Command override |
| `default_models` | Model; a missing key = "Agent default" (pass no model) |
| `provider_surfaces` | Per-agent surface, used only when open mode is per-agent |
| `hidden_providers` | Disabled agents (the row's enable switch) |
| `default_provider` | Default agent |

Add one nested struct (all fields `#[serde(default)]`, maps keyed by `Kind::id()`):

```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AgentPreferences {
    /// "chat" | "tui" | "per-agent". Empty only before migration.
    pub open_mode: String,
    /// Global permission level id ("own", "ask", "edits", "auto", "full"). None only before migration.
    pub permission: Option<String>,
    /// Per-agent permission override; a missing key follows `permission`.
    pub permissions: HashMap<String, String>,
    /// Per-agent effort; a missing key = agent default.
    pub efforts: HashMap<String, String>,
    /// Per-agent argument line as typed; validated by shell-split before it is saved.
    pub args: HashMap<String, String>,
    /// Per-agent environment, `KEY=VALUE` lines, validated before it is saved.
    pub env: HashMap<String, String>,
}
// Preferences: `#[serde(default)] pub agents: AgentPreferences`
```

Every field is a map, string or option, so a stray value cannot fail the whole `Workspace` parse.
Unknown level ids and unknown agent ids are ignored on read (same tolerance as `known_surfaces`).

**Effective config, one accessor.** `Preferences::agent(kind) -> AgentConfig { command, model,
effort, permission, args: Vec<String>, env: Vec<(String, String)>, surface, enabled }` resolves the
global/override rules. Every launch path, the page, the summary line and the launch preview read
it, so the scattered maps stay private to `preferences.rs`.

**Retired behaviour.** `default_access`, `provider_efforts` and `project_providers` stay in the
data untouched (a downgrade still reads them) but nothing reads or writes them after migration:
remove `remember_access` (`conversation.rs:4100`, callers `:2535`, `quick_composer.rs:138`), the write
in `set_effort_choice` (`conversation.rs:2607`), their copy in `picker_preferences.rs:13-23`, the two
direct readers in `app_views.rs:587,657`, and the `project_providers` reads in `hidden_for`,
`new_chat_choice`, `quick_composer`, `automations`.

## 3. Migration (one time, `Workspace::restore`)

Trigger: `saved["preferences"]["agents"]["open_mode"]` is absent. Same absent-key pattern as
`empty_session` (`workspace.rs:586`). Idempotent: once saved, the key exists.

1. `efforts` ← `provider_efforts`, filtered to known agent ids.
2. `permission` ← `default_access` mapped to a level (`supervised`→ask, `auto-accept-edits`→edits,
   `auto`→auto, `full-access`→full, `read-only`→ask); `None` → "own".
3. `open_mode` ← all agents' surfaces (missing = chat): all equal → that value; otherwise
   "per-agent", keeping `provider_surfaces` as is.
4. If `default_provider` is in `hidden_providers`, clear it.
5. `project_providers`: not copied anywhere.

Tests in `preferences.rs`/`workspace.rs`: old blob → new shape for each rule, re-parse is a no-op,
a type error in one new field does not reset the workspace.

## 4. Permission levels

New core enum (presentation-free):

```rust
pub enum Permission { Own, Ask, AcceptEdits, Auto, Full }
```

Session settings carry it through a new `AccessMode::AgentDefault` variant (id "own"), so every
existing `match` on `AccessMode` (`codex()`, `claude()`, ACP mapping, OpenCode rules, TUI build)
must handle it — the compiler finds them all. `ReadOnly` stays composer/plan-only and is not a
settings level. Downgrade note: an older build reading a session saved with "own" will fail to
parse that session's settings; acceptable, stated in the release notes of the native release.

Mapping (`agent::AccessMode` + `tui_launch`):

| Level | Claude chat & TUI | Codex chat (app-server) | Codex TUI | ACP agents | OpenCode | Pi / Omp |
|---|---|---|---|---|---|---|
| own | no `--permission-mode` | no approval/sandbox params | no flag | no `set_mode`, client asks every request | no session rules | — |
| ask | `--permission-mode` default/manual (*) | approval + sandbox as `Supervised` today (*) | `-a on-request -s read-only` | existing Supervised mapping | `*:ask` | — |
| accept edits | `acceptEdits` | as `AutoAcceptEdits` | `-a on-request -s workspace-write` | existing | `edit:allow` | — |
| approve for me | `auto` | as `Auto` | `--approve-for-me` | existing | edit+read allow | — |
| full access | `bypassPermissions` + `--allow-dangerously-skip-permissions` | as `FullAccess` | `--dangerously-bypass-approvals-and-sandbox` | existing | `*:allow` | — |

Decisions after the child 1 review (2026-11-24):
- OpenCode "own" still sends the single rule `question: allow`, so its question tool keeps working.
- Codex TUI "approve for me" uses `-a on-request -s workspace-write -c approvals_reviewer="auto_review"`
  (same setting as the chat path; older CLIs without `--approve-for-me` still start).
- Codex TUI sessions that existed before this change are treated as "own": their stored
  `Supervised` was never chosen by the user. Only new sessions take the settings permission.
- The model catalog probe and Claude rewind spawns get the user's env (not the args).

Pi and Omp have no permission mechanism today: the permission field is hidden for them.
Cursor and Fx support only client-side approval: they show own / approve for me / full access.

(*) Known risk to verify during implementation against the installed CLIs: Claude 2.1.331 lists
`manual` instead of `default`; Codex 0.172.0 `-a` no longer lists `untrusted`. Check acceptance on
the installed versions and fix the mapping if rejected (keep older CLIs working).

## 5. Arguments and environment

**Parsing (core, pure).** `shlex` (already a dependency) splits the line; `None` = unclosed
quote → validation error, previous value stays. `extract_structured(kind, args)` removes known
permission / `--model` / `--effort` flags and returns what it moved, so the page can move them into
their fields and show the notice. `parse_env(text)` rejects lines that are not `KEY=VALUE`
(blank lines and `#` comments allowed).

**Chat injection** (`provider.rs` `run_operation`, after the per-kind `match`, before
`current_dir`; OpenCode in `opencode::command()`):
- args: inserted before the transport subcommand for agents that launch one (`codex app-server`,
  `<cli> acp`, Grok's `agent … stdio`, OpenCode `serve`), so they act as global CLI options;
  Claude/Pi/Omp append them before their own trailing arguments. If a CLI rejects an option, the
  session fails with the CLI's own message (same as a bad flag in a terminal).
- env order (later wins): GUI env → app `settings.environment` → **user env** → account
  (`account.apply`) → per-kind internals. So the account wins on collisions (PRD).
- The warm-process key (`parked.rs:296`) must include args and env so an edit starts a fresh process.

**TUI injection** (`tui_launch::build` / `tui_session::prepare`): args before the Claude prompt
(and its `--`); env = user env then `profile.env(kind)` on top. Structured flags only where
verified (Claude today; Codex once its flags are checked) — controlled by `tui_caps`.

**Collision marker.** `accounts::Profile::owned_keys(kind)` lists the keys the selected account
sets or removes; the env editor marks those rows "Set by the current account".

**Secrets exception.** `.trellis/spec/core/persistence-and-compat.md` says never to store tokens in
plain JSON settings. This task deliberately stores env values in plain text in the workspace JSON
(PRD decision, 2026-11-24). Mitigations: values whose name contains KEY/TOKEN/SECRET/PASSWORD/AUTH
are masked in the read view and in the launch preview, and never logged. Moving them to the OS
credential store is a follow-up task. Record this exception in the spec when the task finishes.

## 6. CLI path, open mode, picker

- `cli::initialize` drops its `INIT` `OnceLock` guard and overwrites the map on every call
  (`cli.rs:15-20`); `apply_preferences` already calls it. The "Restart required" status and the
  CLI details popover (`management.rs` `Target::Cli`) go away; their inspection logic
  (`inspect_cli`, version, reveal) moves into the Command field.
- One function resolves the surface: `open_mode` chat/tui → that; per-agent → `provider_surfaces`
  (missing = chat); remote projects → chat (`surface.rs:63`). It replaces the direct readers
  (`provider_prompt.rs:59`, `:80`, `automations/editor.rs:164`).
- Picker rule (`provider_prompt.rs:36`): prompt when at least two agents are available and any
  available agent resolves to TUI. Update the doc comment and `HINT`.

## 7. UI

Rules: `.trellis/spec/native/component-guidelines.md`. Colours only from `theme.rs` functions;
every interactive element has an id, a `Role`, an English `aria_label`, and a `debug_selector`
where tests need it. Existing widget metrics win over the artboard's where they differ (toggle
36×20, `rounded_xl` card, row padding): the artboard defines structure, states and colour roles,
not new metrics.

**Reused as is** (`settings/widgets.rs`, `popover.rs`, `theme.rs`, `input.rs`): `group`, `row`,
`toggle`, `select` (model, with "Agent default" and "Custom model…"), `secondary_button`,
`primary_button`, `icon_action`, `refresh_button`, `Menu` + `menu_popover` for the row's ⋯ menu,
`Input::field` / `multiline_field`, `right_panel::spinner`, `provider_icon`,
`cx.write_to_clipboard`, `cx.reveal_path`.

**Permission picker.** List = `Menu` with `MenuStyle::Access` (icon, title, description, filled
selection), plus one extension: `MenuAction::detail(text)` renders a second, monospace line (the
flag this agent receives). Trigger = the `select` trigger look with a tinted icon tile.
Colours: new `theme::permission_color(level)` light/dark pairs — own = `content(0.6)`, ask =
`accent_color()`, accept edits = teal, approve for me = violet, full access = `danger_color()`.
Icons: one `permission_icon(level)` shared function. Full access adds an inline warning in
`danger_color()`.

**New shared pieces** (added to the shared layer, each a stateless fn or `RenderOnce` in
`settings/widgets.rs` unless noted, used by this page first):

| Piece | Used for | Built from |
|---|---|---|
| `segmented` icon option | row Chat/TUI | extend existing `segmented` (closes the `providers.rs:224` gap) |
| `radio_cards` (regular + compact) | open mode cards, "Opens in" | `segmented` roles (`RadioGroup`/`RadioButton`) + `modal::radio_row` marker |
| `badge(text, icon, tint)` | Default, Off, permission badges, argument chips | ink/accent/permission tints at 0.13 |
| `level_picker` | effort ladder | `segmented` family, bar glyph drawn with divs |
| `field_grid` / `field` | expanded panel: 136px label column | `row` typography |
| `status_line` | command detection dot, path, version | `success_color` / `warning_color` |
| `saved_flash` | "Saved" after a field commits | `success_color`, timer like `reveal_setting` (1.4 s) |
| `code_block` with Copy | launch preview | `management.rs:502` mono box + clipboard |
| `inline_banner` | detection error with Retry, full-access warning | `danger_color()` tint |
| `skeleton_row` | detection in progress | `content(0.08)` bars |
| `env_table` | read view, masked values, eye toggle, account marker | `icon_action`, mono |
| Settings toast (undo) | flag moved, default cleared, open mode switched, full access | `notifications/surface.rs` `toast_frame`; one at a time, 5 s, optional Undo |

Missing icons are added to `assets.rs` `icon_body` as Lucide stroke bodies (1.6 stroke, matching
the existing family): `hand`, `shield-check`, `shield-off`, `eye-off`, `download`, `star-off`,
`book-open`, `arrow-right-left`. Use existing names where they exist (`more-horizontal`,
`pencil-line`, `settings`, `terminal`, `circle-alert`).

**Text fields** save on blur through `cx.on_blur(&focus, …)` (the `Input` entity has no blur
event). Validation errors render under the field in `danger_color()` with `Role::Alert`.

**Search.** `INDEX` entries for the new rows (`agent-open-mode`, `agent-permission`, and per-agent
landing on `settings-agent-<id>` via `reveal_probe`); keywords: permission, approval, full access,
yolo, arguments, args, env, environment, open in, terminal, tui, chat, effort, model, command,
path, install, update.

**Page structure** (`providers_page`): Accounts, Usage and privacy, Background tasks, Advanced
stay. Agent CLIs = defaults card (open mode cards, permission picker) + "Installed" group
(count, checked time, Refresh) + "Available to install" group. The scope select and every
`project_providers` write are removed.

## 8. Compatibility and rollback

- Old saved state loads (absent keys → migration). New state read by an old build: new keys
  ignored; old fields still hold sensible values (only `default_access`/`provider_efforts` stop
  changing). Sessions saved with permission "own" do not load in an older build (§4).
- Rollback = revert the commits; data needs no cleanup.

## 9. Tests (one pass per child task, filtered to touched modules)

- Core: permission mapping per agent incl. "own" (no flag), args split/extract, env parse, TUI
  flag order with args before the prompt, chat env order (account wins), reloadable `cli::initialize`.
- GPUI pure: migration rules, accessor (global vs override, disabled default), surface resolver,
  picker rule.
- GPUI layout (`settings/layout_tests.rs`): rewrite the two surface tests; add expand-one-row,
  permission pick writes prefs, flag moved out of arguments, invalid args keep the old value,
  disabling the default clears it. Rewrite `conversation.rs:4987`, `:5018` to "session-only".
- Linux-portable: `secondary-` keystrokes, no macOS paths.
