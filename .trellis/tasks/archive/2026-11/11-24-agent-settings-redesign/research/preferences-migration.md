# Research: Preferences data model, call sites and migration for per-agent config

- **Query**: Agent-related `Preferences` fields, persistence/migration mechanics, every reader/writer of the
  agent fields, the desktop localStorage import, existing tests, and a proposed per-agent data model +
  one-time migration plan.
- **Scope**: internal (crates/monocode-gpui, crates/monocode-core, .trellis/spec)
- **Date**: 2026-11-24

All paths are relative to `crates/` unless noted. Line numbers are from the working tree on
`chore/adopt-trellis` (commit 531d9e31).

---

## 1. `Preferences` agent fields, serde and persistence

### 1.1 Fields (`monocode-gpui/src/settings/preferences.rs`)

The struct is `#[derive(Serialize, Deserialize)] #[serde(default)]` (`:20-22`); no `rename_all`, so
JSON keys are the snake_case field names (except `cloudKit`, `cloudAutoStop`, `:132`, `:135`).

| Field | Type | Line | Default (`:203-216`) | Notes |
|---|---|---|---|---|
| `binary_paths` | `HashMap<String,String>` | `:102` | empty | CLI path override per provider id |
| `provider_surfaces` | `HashMap<String, Surface>` | `:104-105` | empty | `#[serde(deserialize_with = "known_surfaces")]` drops unknown provider ids / surface values (`:9-18`) |
| `default_provider` | `Option<String>` | `:108` | `None` | global default agent |
| `default_models` | `HashMap<String,String>` | `:109` | empty | global model per provider |
| `hidden_providers` | `Vec<String>` | `:110` | empty | "Show in picker" off, global |
| `project_providers` | `HashMap<String, ProviderDefaults>` | `:111` | empty | per-project `{provider, models, hidden}` (`ProviderDefaults` `:296-302`, also `#[serde(default)]`) |
| `favorite_models`, `recent_models` | `Vec<ModelChoice>` | `:112-113` | empty | `recent_models[0]` is step 3 of the new-chat resolver |
| `model_picker_tab` | `Option<String>` | `:114` | `None` | |
| `default_access` | `Option<String>` | `:115-116` | `None` | doc: "The last access mode picked in a composer; new chats start with it." |
| `provider_efforts` | `HashMap<String,String>` | `:117-118` | empty | doc: "Last explicit effort choice for each provider, shared across projects." |
| `background_model` | `Option<ModelChoice>` | `:121` | `None` | not in scope |
| `claude_hooks`, `claude_entrypoint` | `bool` | `:99-101` | true / false | Claude-specific, not in scope |

Helper: `Preferences::hidden_for(project)` (`:317-325`) = `hidden_providers` ∪
`project_providers[project].hidden`, sorted + deduped. This is the single place that merges per-project
hiding; most picker call sites go through it.

### 1.2 How missing / unknown fields deserialize

- Missing field → value from `impl Default for Preferences` (struct-level `#[serde(default)]`).
  Test proof: `auto_resume_usage_limit_is_off_by_default_and_survives_old_files` (`:333`) parses
  `{"sounds":false}`.
- Unknown field → ignored (no `deny_unknown_fields`). Test proof: `cloud_kit_is_off_by_default...`
  (`:347`) parses `{"cloudKitEnabled":true}`.
- A *type-mismatched* field fails the whole `Preferences` deserialize, which in turn fails
  `serde_json::from_str::<Workspace>` and `Workspace::restore` silently falls back to
  `unwrap_or_default()` (`monocode-gpui/src/workspace.rs:573-576`) — i.e. the entire workspace (tabs,
  projects, all preferences) resets. That is why `provider_surfaces` uses the tolerant
  `known_surfaces` deserializer ("must not cost the user every other preference", `:7-8`).
- Spec: `.trellis/spec/core/persistence-and-compat.md` "Serde conventions": tolerant reading with
  `#[serde(default)]`, never `deny_unknown_fields` on stored data, rename via `#[serde(alias = ..)]`.
  `.trellis/spec/native/state-and-async.md:52-53` "Adding a preference": add field + default; both
  structs are `#[serde(default)]` so old saved state loads.

### 1.3 Versioning / migration mechanism

There is **no version number** on `Workspace` or `Preferences`. Migrations are ad hoc, in
`Workspace::restore` (`monocode-gpui/src/workspace.rs:573-...`), keyed on the *absence* of a new
key in the raw JSON:

- `:579-583`: if `new_layout_boxy` is absent, derive it from `classic_layout`.
- `:586-591`: if `preferences.empty_session` is absent and legacy `preferences.empty_session_games`
  is a bool, map it to `"games"`/`"none"`.
- `:592-597`: if `project_rail_pinned` is absent, run `restore_project_rail_metadata()`.
- `Preferences::terminal_placement()` (`preferences.rs:230-236`) is a read-time fallback on legacy
  `terminal_in_panel`, and `set_terminal_placement` keeps both in step (`:238-241`) — a "dual
  field" pattern.

Pattern to copy: parse `saved` as `serde_json::Value`, test `saved["preferences"].get("<new key>")
.is_none()`, then seed the new field from the old ones. Once saved, the new key exists and the
migration never runs again (idempotent, no version needed).

### 1.4 Storage path

- `Store::ui_state()` reads `native_settings` key `'workspace'` (`monocode-core/src/store.rs:303-317`);
  `save_ui_state` upserts it (`store.rs:546-549`). For the production shared (desktop) database both
  delegate to `shared_ui_state` / `shared_save_setting("workspace", ..)` which store in the sidecar
  `<db>.gpui.sqlite3` (`store/shared.rs:536`, spec persistence-and-compat.md:21-23).
- **`shared_ui_state` re-applies desktop localStorage on every read**:
  `preferences.apply_workspace(&mut merged)` (`monocode-core/src/store/shared.rs:2464-2466`). See §3.
- Load: `Shell::new` → `Workspace::restore(saved, ..)` (`monocode-gpui/src/shell.rs:1143`), then
  `monocode_core::cli::initialize(&workspace.preferences.binary_paths)` (`shell.rs:1144`).
  `cli::initialize` is a `OnceLock` (`monocode-core/src/cli.rs:15-20`) — hence "command path change
  takes effect after restart".
- Save: `Shell::persist` (`shell.rs:4750`) → `save_state` (`shell.rs:4780-4800`) serializes the whole
  `Workspace` and writes it on the background executor, guarded by a generation counter. Also a
  quit hook writes `latest_state` (`shell.rs:1145-1156`).
- Settings edits: `SettingsView::update` (`settings/mod.rs:687-692`) mutates its `prefs` copy and
  emits `SettingsEvent::Changed(Box<Preferences>)`; `shell/settings.rs:221-230` assigns
  `workspace.preferences = *prefs` and calls `apply_preferences`, which (among others) calls
  `cli::initialize(binary_paths)` again (`shell/settings.rs:552`, no-op after first) and
  `cx.set_global(self.workspace.preferences.clone())` (`shell/settings.rs:567`).
- Composer-side writes go to the *global* `Preferences` (`cx.update_global`), and
  `observe_picker_preferences` (`shell/picker_preferences.rs:5-28`) copies
  `model_picker_tab, favorite_models, recent_models, file_view_mode, default_access,
  provider_efforts` back into `workspace.preferences`, then `sync_settings` + `persist`.

---

## 2. Every reader / writer of the agent fields

R = read, W = write. Test-only sites are listed in §4.

### 2.1 `binary_paths` → per-agent **command**

| Site | R/W | What it does |
|---|---|---|
| `monocode-gpui/src/shell.rs:1144` | R | `cli::initialize(&binary_paths)` at startup |
| `monocode-gpui/src/shell/settings.rs:552` | R | same in `apply_preferences` (OnceLock, so no effect after first) |
| `monocode-gpui/src/settings/management.rs:55-65` | R | prefill the CLI path input of the CLI management dialog |
| `monocode-gpui/src/settings/management.rs:164-171` | W | after inspection succeeds, insert/remove path |
| `monocode-gpui/src/settings/management.rs:467-471` | R | "overridden" and "pending restart" (compares with `cli::configured`) |
| `monocode-gpui/src/settings/management.rs:983` | R | shows "Use auto-detected path" button |
| `monocode-core/src/desktop_preferences/workspace_preferences.rs:249-263` | W (import) | reconciles from `monocode.providerBinaryPaths.v1` |

### 2.2 `provider_surfaces` → per-agent **surface** (Chat/TUI)

| Site | R/W | What it does |
|---|---|---|
| `monocode-gpui/src/shell/provider_prompt.rs:59-69` | R | `Shell::provider_surface(kind)`: the preference, or Chat when DB cannot store TUI |
| `monocode-gpui/src/shell/provider_prompt.rs:79-92` + `:29-37` | R | `should_prompt_for_provider`: prompt only if *default* agent is TUI and ≥2 available |
| `Shell::provider_surface` callers | R | `shell.rs:4241`, `shell.rs:5280`, `shell/app_views.rs:745`, `shell/automation_runs.rs:365`, `shell/provider_prompt.rs:154`, `:200` |
| `monocode-gpui/src/automations/editor.rs:164-167` | R | `runs_in_terminal` for automations |
| `monocode-gpui/src/settings/pages/providers.rs:224-249` | R/W | segmented Chat/TUI control per agent |
| `monocode-core/src/desktop_preferences/workspace_preferences.rs:264-277` | W (import) | from `monocode.providerSurface.v1` |
| `monocode-core/src/surface.rs:63-79` | — | `new_session_surface` (remote → Chat) / `automation_surface` consume the preferred value |

Change (d): a global open mode `chat | tui | per_agent` must be resolved in **one** function that
replaces direct map reads; today the direct readers are `provider_surface()` (`provider_prompt.rs:59`),
`should_prompt_for_provider()` (`:80-85`, passes the raw map), and `automations/editor.rs:164`.
The PRD's new prompt rule ("any available agent set to TUI") replaces `should_prompt_for_provider`
(`provider_prompt.rs:29-37`), which currently only checks the default agent.

### 2.3 `default_models` → per-agent **model**

| Site | R/W | What it does |
|---|---|---|
| `monocode-gpui/src/shell/new_chat_choice.rs:28-35` | R | `default_model`: project model, else `default_models` |
| `monocode-gpui/src/quick_composer.rs:362-367` | R | `set_project`: project model, else global |
| `monocode-gpui/src/automations.rs:359-365` | R | automation draft model: project, else global |
| `monocode-gpui/src/monos/mod.rs:899` | R | Mono chat model |
| `monocode-gpui/src/settings/pages/providers.rs:108-112` | R | selected model; falls back to first catalog model |
| `monocode-gpui/src/settings/pages/providers.rs:205-221` | W | model select (global or project scope) |
| `monocode-core/src/desktop_preferences/workspace_preferences.rs:293-301` | W (import) | from `monocode.defaultModels` via `native_models` |
| indirect via `NewChatChoice::explicit/resolve` | R | `shell/mobile_host.rs:702` (`mobile_defaults`), `:3611-3615` (`explicit_model`), `shell/app_views.rs:581`, `:650-655`, `shell/quick_composer.rs:34`, `shell/session_loading.rs:146` |

Note `providers.rs:163-197`: "Use by default" is disabled when `selected_model.is_none()` — the bug
the PRD's "Agent default" model option fixes.

### 2.4 `hidden_providers` → becomes the **enable** switch

| Site | R/W | What it does |
|---|---|---|
| `preferences.rs:317-325` `hidden_for` | R | global ∪ project hidden |
| `hidden_for` callers | R | `new_chat_choice.rs:39` (`allowed`, skipped for remote), `provider_prompt.rs:72-76`, `shell/contributors.rs:145` (Ask dialog), `shell/mobile_host.rs:3657-3662` (phone model sheet, remote → none), `quick_composer.rs:537-539`, `conversation.rs:3090-3092` (composer model list), `conversation.rs:3181-3183` (recent model items), `conversation/mono.rs:1005-1007`, `conversation/response_menu.rs:34-36` |
| `monocode-gpui/src/shell/settings.rs:216-219` | R | `CheckHarnessUpdates`: skip update checks for hidden agents |
| `monocode-gpui/src/shell/notifications.rs:56` | R | `HarnessUpdates::new(hidden_providers)` at startup |
| `monocode-gpui/src/settings/pages/providers.rs:99-101` | R | `globally_hidden`, `locked` (project scope), `visible` |
| `monocode-gpui/src/settings/pages/providers.rs:268-284` | W | "Show in picker" toggle, global or project `hidden` |
| `monocode-core/src/desktop_preferences/workspace_preferences.rs:302-311` | W (import) | from `monocode.hiddenPickerProviders` |

Unrelated name clash: `monocode-core/src/session_filters.rs:70-100` `Filters::hidden_providers` is the
sidebar session filter (`workspace.session_filters`), a different field; do not touch it.

Change (a): if "hidden" becomes "disabled", `hidden_for` must stop merging `project_providers[..].hidden`
(per PRD (c)), and the default resolver must skip disabled agents (already does via `allowed`), plus
the "disabling the default clears the default" rule must be added at the write site.

### 2.5 `provider_efforts` → per-agent **effort** (seed only)

| Site | R/W | What it does |
|---|---|---|
| `monocode-gpui/src/conversation.rs:2601-2615` `set_effort_choice` | **W** | composer effort pick writes the global map ("remember last") |
| `monocode-gpui/src/conversation.rs:4113-4116` `preferred_effort` | R | lookup |
| `preferred_effort` callers | R | `conversation.rs:813` (new Conversation settings), `conversation.rs:2845` (reset after model/provider change), `quick_composer.rs:327`, `:374`, `:1775`, `shell/app_views.rs:586`, `:656`, `shell/session_loading.rs:154`, `shell/mobile_host.rs:2569`, `:2991`, `shell/mobile_host/cloud_start.rs:158` |
| `monocode-gpui/src/shell/picker_preferences.rs:14`, `:23` | R/W | copies global → workspace and persists |

Change (b): remove the write in `set_effort_choice` (`conversation.rs:2607-2614`) and the copy in
`picker_preferences.rs:14/23`; repoint `preferred_effort` to the new per-agent effort. Callers keep
their signature if `preferred_effort` is kept as the accessor.

### 2.6 `default_access` → global **permission** (seed only)

| Site | R/W | What it does |
|---|---|---|
| `monocode-gpui/src/conversation.rs:4094-4099` `default_access` | R | `AccessMode::from_id`, default `AccessMode::Supervised` (`monocode-core/src/agent.rs:7-14`) |
| `monocode-gpui/src/conversation.rs:4100-4106` `remember_access` | **W** | sets `default_access` |
| `remember_access` callers | W | `conversation.rs:2533-2535` (composer `access:` menu), `shell/quick_composer.rs:138` (quick composer launch) |
| `default_access()` callers | R | `conversation.rs:812`, `shell/quick_composer.rs:39`, `:47`, `shell/session_loading.rs:153`, `shell/mobile_host.rs:709` (via `remote_default_access`), `shell/mobile_host/cloud_start.rs:157` |
| `monocode-gpui/src/shell/app_views.rs:587-593`, `:657-663` | R | read `workspace.preferences.default_access` directly (not via helper) |
| `monocode-gpui/src/shell/picker_preferences.rs:13`, `:22` | R/W | copy + persist |

Change (b)/(d): drop both `remember_access` call sites and the picker copy; route all readers through
one accessor that takes `kind` (per-agent override, else global). Note the two direct readers in
`app_views.rs` bypass the helper.

### 2.7 `default_provider` → global default agent

| Site | R/W | What it does |
|---|---|---|
| `shell/new_chat_choice.rs:45-55` | R | resolver step 2 |
| `quick_composer.rs:355-361` | R | `set_project` |
| `automations.rs:354-358` | R | automation draft provider |
| `monos/mod.rs:885-889` | R | Mono chat provider (no hidden check) |
| `settings/pages/providers.rs:93-98`, `:180-190` | R/W | "Default"/"Use by default" |
| `monocode-core/src/desktop_preferences.rs:167-179` | W (import) | from `monocode.lastModel.harness` |

### 2.8 `project_providers` → stops applying (c)

| Site | R/W | What it does |
|---|---|---|
| `preferences.rs:319-321` (`hidden_for`) | R | merges project hidden |
| `shell/new_chat_choice.rs:29-32`, `:46-49` | R | project model, resolver step 1 |
| `quick_composer.rs:356-358`, `:363-365` | R | project provider/model |
| `automations.rs:355`, `:361` | R | project provider/model |
| `settings/pages/providers.rs:90-94`, `:186`, `:214-219`, `:279` | R/W | "Provider defaults scope" select (`provider_scope`, `settings/mod.rs:349`, `:508`; select at `providers.rs:58-80`) |
| `monocode-core/src/desktop_preferences.rs:180-220` | W (import) | from `monocode.projectProviderSettings.v1` |

To make (c) true, these five reader groups stop consulting `project_providers`; the data stays (PRD:
"stay in the data but no longer apply"). The scope select on the page goes away with the redesign.

### 2.9 Things that do not exist yet

- No launch-args or env-vars concept anywhere in core/gpui (grep `launch_args|extra_args|env_vars|extra_env`
  hits only Mono bridge/visual bridge constants and a terminal test).
- No Codex history import path preference (only `CODEX_HOME` handling in `monocode-core/src/accounts.rs:113`,
  `mcp.rs:436`).
- No "use the agent's own settings" permission: `agent::Settings.access` is a non-optional
  `AccessMode` (default `Supervised`), and the Claude TUI always passes
  `--permission-mode <access.claude()>` (`monocode-core/src/tui_launch.rs:78`; test
  `access_maps_to_the_permission_mode` `:482`). A "no flag" option needs `Option<AccessMode>` (or a new
  variant) through `LaunchInput` and the chat providers (`provider/acp.rs:78`, `:350`).

---

## 3. Desktop (React) localStorage import — `workspace_preferences.rs`

`DesktopPreferences::apply_workspace` (`monocode-core/src/desktop_preferences.rs:65-226`) +
`apply_preferences` (`desktop_preferences/workspace_preferences.rs:6-...`) map `monocode.*`
localStorage keys onto the native workspace JSON field names, using a 3-way merge
`reconcile_preference(target, baseline, next, field, source)` (`desktop_preferences.rs:412-422`,
`reconcile_value` `:426-...`): baseline = last-seen desktop value stored in
`workspace.desktop_project_preferences`; a native edit is kept unless the desktop value changed since
the baseline; map keys are merged individually.

It writes into: `default_provider` (`desktop_preferences.rs:171-178`), `project_providers` (`:181-219`),
`binary_paths` (`workspace_preferences.rs:263`), `provider_surfaces` (`:277`), `default_models`
(`:293-301`), `hidden_providers` (`:302-311`). It does **not** touch `provider_efforts` or
`default_access`.

It runs on **every** `shared_ui_state()` read (`store/shared.rs:2464-2466`), only when the data dir is
the real desktop app's (`DesktopPreferences::load`, `desktop_preferences.rs:40-51`).

Consequences for the redesign:

- **If the existing fields stay the storage** (extend-maps model), the importer needs **no change**: it
  keeps writing the same field names with the same meaning (path, surface, model, hidden ≈ disabled).
- **If a new `agents` map replaces them**, then either (i) the importer must also write into the new map
  (a core change, plus new tests), or (ii) the native side must keep reading/merging the old fields
  every load (not one-time), otherwise a later desktop edit lands in a field nobody reads. Also a
  one-time migration that runs before `apply_workspace` would be overwritten on the next read only for
  fields the importer owns.
- `project_providers` keeps being imported; harmless once nothing reads it.
- The PRD scopes the React page out; nothing in the importer needs to know about args/env/permission.

---

## 4. Existing tests touching these fields

### Unit tests (pure, fast)

| Test | Location | Covers |
|---|---|---|
| `auto_resume_usage_limit_is_off_by_default_and_survives_old_files` | `settings/preferences.rs:333` | missing-field default pattern |
| `cloud_kit_is_off_by_default_and_round_trips_by_id_only` | `preferences.rs:347` | unknown-field tolerance, round trip |
| `surfaces_round_trip_and_default_to_none` | `preferences.rs:375` | `provider_surfaces` JSON shape `{"claude":"tui"}` |
| `unknown_providers_and_surfaces_are_dropped_without_losing_other_preferences` | `preferences.rs:391` | `known_surfaces` |
| `nothing_set_starts_codex` … `explicit_provider_takes_the_project_then_default_model` | `shell/new_chat_choice_tests.rs:39-130` | resolver order, project beats settings, hidden skipping, remote ignores hidden |
| `remote_projects_keep_hidden_providers_in_the_model_sheet` | `shell/mobile_host.rs:~4971` | `hidden_for` incl. project hidden (`:4983-4995`) |
| `an_explicit_phone_provider_without_a_model_uses_its_configured_model` | `shell/mobile_host.rs:~5000` | `default_models` + `project_providers` (`:5002-5050`) |
| `access_maps_to_the_permission_mode`, `flag_order_is_model_mode_settings_sources_session_prompt` | `monocode-core/src/tui_launch.rs:482`, `:497` | always-on `--permission-mode` |
| `desktop_provider_choices_seed_defaults_and_preserve_explicit_native_edits` | `monocode-core/src/desktop_preferences.rs:~694` | `default_provider`, `project_providers` import |
| `provider_surfaces_import_only_known_providers_and_values` | `desktop_preferences.rs:~722` | surfaces import |
| `desktop_settings_and_sidebar_filters_use_native_names_units_and_enums` | `desktop_preferences/workspace_preferences.rs:~741` | `default_models`, hidden filters |
| `unrelated_desktop_map_edits_preserve_native_model_choices_and_added_keys` | `workspace_preferences.rs:~805` | per-key merge of `default_models` |
| `workspace.rs:1522`, `:2020` | `workspace.rs` | examples of restore-time migrations (`legacy_boxy_layout_...`, `state_saved_by_an_older_version_...`) |

### GPUI tests (slow; `monocode-gpui` test binary)

| Test | Location | Covers |
|---|---|---|
| `picked_access_becomes_the_default_for_new_chats` | `conversation.rs:4987` | "remember last access" — **behaviour being retired** |
| `effort_choices_survive_provider_switches_and_new_projects` | `conversation.rs:5018` (asserts `provider_efforts`, `default_access` `:5038-5040`) | "remember last effort" — **being retired** |
| `favorite_models_reach_the_workspace_and_the_saved_state` | `shell/picker_preferences.rs:38` | picker observer persistence |
| surface suite | `shell/surface_tests.rs:100-302` (`a_new_session_opens_as_its_providers_preferred_surface`, `..._in_a_folder_follows_the_default_providers_surface`, `a_split_follows_...`, `the_new_session_command_skips_the_prompt_when_one_provider_is_installed`, `a_database_that_cannot_store_terminals_...`, `a_handoff_or_second_opinion_is_a_chat_...`); helper `all_terminal` `:44` writes `provider_surfaces` |
| `each_provider_chooses_the_surface_its_new_sessions_open_in` | `settings/layout_tests.rs:199` | settings page writes `provider_surfaces` |
| `the_dialog_excludes_a_provider_hidden_for_the_project` + `window` helper | `shell/provider_prompt.rs:~551-640` | prompt + project hidden |
| `without_a_chosen_provider_the_project_default_is_used`, `the_dialog_offers_the_providers_the_picker_shows_with_the_project_default` | `shell/contributors_tests.rs:917`, `:1036` | `project_providers` in Ask dialog — **affected by (c)** |
| `ordinary_shell_new_chat_follows_the_last_choice` | `shell/new_chat_choice_tests.rs:165` | |
| `the_editor_knows_when_its_automation_runs_in_a_terminal` | `automations.rs:1367` | surfaces in automations |
| `a_clean_exit_never_replaces_the_last_tab_with_another_terminal` | `shell/tui_sessions/tests.rs:392` | sets surfaces |
| inbox/app_views helpers | `shell/app_views.rs:860-1016`, `shell/today_sessions.rs:187`, `monos/emoji_library_tests.rs:945-1006`, `shell/automation_runs/tui.rs:203-208` | set `default_provider`/`provider_surfaces` as fixtures |

### Where new tests fit

- Migration + serde of the new model: `settings/preferences.rs` `mod tests` (pure; parse an old JSON
  blob with `binary_paths/default_models/hidden_providers/provider_surfaces/provider_efforts/
  default_access` and assert the new shape; re-parse saved output to prove idempotence). If the
  migration lives in `Workspace::restore`, test it next to `workspace.rs:1522`.
- Effective-config resolution (open mode, permission override, enabled/default fallback): a pure
  function tested in `shell/new_chat_choice_tests.rs` (resolver) or a new `settings/agents_tests.rs`.
- Args shell-split + flag extraction, env parsing/masking: pure unit tests in core (e.g. next to
  `tui_launch.rs` tests) — fast, run in the Linux/Windows core CI shards.
- TUI arg wiring: extend `tui_launch.rs` tests (`flag_order_...`, `access_maps_...`).
- New-session prompt rule: `shell/surface_tests.rs` / `shell/provider_prompt.rs` tests.
- The two retiring GPUI tests in `conversation.rs:4987`, `:5018` must be rewritten to the new
  semantics (session-only change, new session starts from settings).

---

## 5. Data model options and recommendation

### Option A — new `agents: HashMap<String, AgentConfig>`

```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AgentConfig {
    pub command: String,            // dup of binary_paths
    pub model: Option<String>,      // dup of default_models; None = "Agent default"
    pub effort: Option<String>,
    pub permission: Option<String>, // None = inherit global
    pub args: String,
    pub env: String,
    pub surface: Option<Surface>,   // dup of provider_surfaces
    pub enabled: bool,              // dup (inverse) of hidden_providers -- note: Default gives false!
    pub import_path: Option<String>,
}
```

Costs found:
- Four fields duplicate data the desktop importer keeps writing every read (§3) → core importer change
  or per-load merge; otherwise desktop edits are lost silently.
- `binary_paths` must still be handed to `cli::initialize` (`shell.rs:1144`) and the CLI management
  dialog (`management.rs`), so either a projection or more call-site edits.
- `enabled: bool` with `#[serde(default)]` defaults to `false`; needs a custom default or the inverted
  `disabled` meaning — a classic trap with `#[serde(default)]` on a new struct.
- A whole struct per agent means a type error in any sub-field drops the whole Workspace on load
  (§1.2) unless each field gets a tolerant deserializer.
- Touches every reader in §2.1-2.4 (≈35 sites + tests).

### Option B — keep the existing maps, add the missing per-agent maps (recommended)

Keep as-is (same JSON keys, same meaning, importer unchanged):
`binary_paths` (command), `default_models` (model; absence = "Agent default"),
`provider_surfaces` (per-agent surface), `hidden_providers` (= not enabled), `default_provider`.

Add (each `#[serde(default)]`, keyed by `Kind::id()`):

```rust
/// Per-agent effort set on the Agents page. Seeded once from `provider_efforts`.
pub agent_efforts: HashMap<String, String>,
/// Per-agent permission override (access-mode id). Missing = use `agent_permission`.
pub agent_permissions: HashMap<String, String>,
/// Per-agent launch argument line, as typed (validated by shell-split on save).
pub agent_args: HashMap<String, String>,
/// Per-agent environment as `KEY=VALUE` lines.
pub agent_env: HashMap<String, String>,
/// Codex history import path.
pub codex_import_path: Option<String>,
/// "chat" | "tui" | "per-agent". Empty = not migrated yet.
pub agent_open_mode: String,
/// Global permission; "own" = pass no flag. `None` = not migrated yet.
pub agent_permission: Option<String>,
```

Optionally group the new ones in one nested struct (`agents: AgentSettings { open_mode, permission,
efforts, permissions, args, env, codex_import_path }`) to keep `Preferences` readable; a nested
`#[serde(default)]` struct with only map/string/option fields is safe.

Why B:
- Back-compat: no existing key changes meaning or type, so old builds reading a new blob and new
  builds reading an old blob both work (unknown keys ignored, missing keys defaulted). The React
  importer and its 4 tests need no change (§3).
- Minimal churn: `binary_paths`/`default_models`/`provider_surfaces`/`hidden_providers` readers keep
  compiling; only semantics change at the few places listed for (a)-(d).
- `provider_efforts` / `default_access` stay as written (frozen legacy), so a downgrade still sees the
  last values; the PRD's "read once" is satisfied by a migration that seeds the new fields only when
  they are absent.
- Matches existing house patterns: per-provider `HashMap<String, _>` keyed by `Kind::id()`, and
  "absent key ⇒ migrate" in `Workspace::restore`.

Cost of B: the per-agent view is assembled from several maps; mitigate with one read accessor
(`Preferences::agent(kind) -> EffectiveAgent { command, model, effort, permission, args, env,
surface, enabled }`) that every launch path uses, so the scatter is confined to `preferences.rs`.

### One-time migration plan (in `Workspace::restore`, `workspace.rs:~585`, same style as `empty_session`)

Trigger: `saved["preferences"].get("agent_open_mode").is_none()` (the marker key; or a dedicated
`agents_migrated: bool`). Run after deserialize, before first save:

1. **Effort**: `agent_efforts = provider_efforts.clone()` (filter to `Kind::parse` ids).
2. **Permission**: global `agent_permission = "own"` per PRD ("existing users see no behaviour
   change"). Caveat for the main agent: today a new chat always gets an access mode —
   `default_access` if set, else `Supervised` (`conversation.rs:4094-4099`) — and the Claude TUI always
   passes `--permission-mode` (`tui_launch.rs:78`). So "own" is *not* byte-identical to today for a
   user whose `default_access` is set (e.g. `"auto"`). If "no behaviour change" is the priority, seed
   `agent_permission = default_access.unwrap_or("own")`. This is a product decision; flag it in
   design.md.
3. **Open mode**: collect `provider_surfaces` values for all `Kind::ALL` (missing = `Chat`). If all
   equal → `"chat"` or `"tui"`; if they differ → `"per-agent"` and keep the map as-is (PRD).
4. **Enabled**: nothing to do (`hidden_providers` keeps its meaning). If the current default is
   hidden, clear `default_provider` (PRD rule) — today `resolve` already skips it.
5. **Project providers**: nothing to migrate; stop reading. Optionally (product call) do not copy
   project hidden lists into the global list.
6. Leave `provider_efforts` and `default_access` untouched (frozen), and remove their writers
   (§2.5/§2.6) so they never change again.

Idempotence: once saved, the marker key exists, so step 1-3 never rerun; a later desktop import
does not write the new keys.

## Related specs

- `.trellis/spec/native/state-and-async.md:30-53` — Workspace/Preferences persistence flow, "Adding a
  preference".
- `.trellis/spec/native/directory-structure.md:57` — settings row → `Preferences` field +
  `SettingsView::update`.
- `.trellis/spec/core/persistence-and-compat.md` "Serde conventions" (tolerant read, alias on rename),
  "Identifiers that never change" (`monocode.*` localStorage keys are read-only from native), and
  "Secrets: ... Never store a token in plain JSON settings."

## Caveats / Not found

- The PRD stores env values in plain text in the settings JSON; the core persistence spec says "Never
  store a token in plain JSON settings". The PRD explicitly accepts this (OS credential store out of
  scope); worth stating in design.md so a reviewer does not flag it.
- No `Workspace`/`Preferences` version field exists; migration relies on absent-key detection.
- `apply_workspace` reconciliation runs on every read only for the shared (desktop) DB; dev/isolated
  DBs never import (`desktop_preferences.rs:36-51`).
- Line numbers for tests in `mobile_host.rs`, `desktop_preferences.rs`, `workspace_preferences.rs` are
  approximate (±3) where marked `~`.
- Not examined: chat-provider side (`provider/*.rs`) handling of model/effort/access per provider
  beyond `acp.rs:78/350`; the design.md support table will need that.
