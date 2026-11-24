# State and Async

> Where state lives in the native app, how it is saved, and how work leaves the UI thread.

## Startup wiring (`src/main.rs`)

`main()` parses flags, opens the `Store` (`monocode_core::store::Store::open_monocode`),
reads the saved UI state (`db.ui_state()`), restores `workspace::Workspace::restore(..)`,
wraps the store as `Arc<Mutex<Store>>`, then in `gpui::Application::run`:

1. installs process-wide services (`ui_store_saves::install`, `image_memory::init`,
   `frame_log::start`, ...),
2. calls every module's `bind(cx)` for keybindings,
3. sets `cx.set_prompt_builder(modal::prompt_builder)`,
4. opens one window whose root is `cx.new(|cx| shell::Shell::new_with_metadata(db, ..))`.

Heavy work that is not needed for the first frame is deferred with `window.on_next_frame` and
run on the background executor (the thread-dir sweep in `main.rs`). Do not add blocking work
to the startup path: "The shell does not query thousands of saved agents on the UI thread".

Development data is isolated: debug builds use `data_dir()` (`Monoptah Development`, or
`MONOCODE_NATIVE_DATA_DIR`), and `main.rs` refuses fixtures (`--fixture`, `--chat-fixture`)
against the real desktop database.

## Where state lives

| State | Holder | Example |
| --- | --- | --- |
| The window and everything in it | `Shell` entity (`shell.rs`), with child `Entity<..>` fields | `sessions: HashMap<i64, Entity<Conversation>>`, `settings: Option<Entity<SettingsView>>` |
| Tabs, panes, projects, docks, preferences (persisted) | `Workspace` (`workspace.rs`), a `#[serde(default)]` struct stored on `Shell` as `self.workspace` | `workspace.preferences: settings::Preferences` |
| App-wide preferences | `settings::Preferences` (`settings/preferences.rs`), `#[serde(default)]` + `impl Default` with the React app's defaults | `p.sounds`, `p.terminal_placement` |
| Process-wide services | GPUI globals: `struct Shared(..); impl Global for Shared {}` with `install(cx)` / `get(cx)` | `accounts.rs` (`Shared(Entity<Accounts>)`), `ui_store_saves.rs`, `notifications/updater.rs`, `settings/cloud_env.rs` |
| Database | `runtime::Database = Arc<Mutex<Store>>`, locked with `runtime::locked(&store)` | returns `Result<MutexGuard<Store>, String>`; never `.lock().unwrap()` in app code |
| Theme tokens | thread-locals in `theme.rs`, set by `configure_project` | read via `content()`, `accent_color()` |

Globals are for genuinely process-wide services. A global install is idempotent
(`accounts::install` returns early when `get(cx)` is `Some`), and per-`App` state such as
`main.rs` `QuitGate` lives on the `App` rather than in a `static` so separate test Apps do
not interfere.

## Persisting state

- **Workspace and preferences** are one JSON blob. `Shell::persist` normalizes the workspace
  and calls `Shell::save_state` (`shell.rs`), which serializes `self.workspace`, stores it in
  `latest_state` (used by quit), and writes it with `store.save_ui_state` on the background
  executor. A monotonically increasing `generation` makes stale writes skip; a failure lands
  on the entity as `this.error = Some("Could not save workspace: ..")`.
- A preference change flows: page widget → `SettingsView::update(cx, |p| ..)` (emits
  `SettingsEvent::Changed(Box<Preferences>)`) → `shell/settings.rs` assigns
  `workspace.preferences`, calls `apply_preferences` (pushes values into core, e.g.
  `monocode_core::cli::initialize`, and re-applies keybindings) and `persist`.
- **Adding a preference:** add the field to `Preferences` with its default in
  `impl Default for Preferences`. Because both structs are `#[serde(default)]`, old saved
  blobs still load. Migrations of renamed or reshaped fields go in `Workspace::restore`
  (it inspects the raw `serde_json::Value`, e.g. `empty_session_games` → `empty_session`).
  Saved entries that name an unknown provider are dropped, not fatal
  (`known_surfaces` in `settings/preferences.rs`).
- **Single latest-value settings saved off-thread** use `ui_store_saves::UiStoreSaves`
  (`save_latest(value)` returns a `SaveTicket`; `drain()` waits for the accepted value).
  `main.rs` `quit_after_saving` drains it before stopping processes and keeps the app open if
  the drain fails.
- Do not rename `monocode.*` storage keys or desktop-preference keys
  (`workspace.rs` `restore_project_rail_metadata` reads `monocode.railMonosPinned`); see the
  "Never rename these" table in `CLAUDE.md`.

## Background work

The standard shape (used by `Shell::save_state`, `SettingsView::refresh` in
`settings/mod.rs`, and ~200 other call sites):

```rust
let work = cx.background_executor().spawn(async move { data::discover(&cwd) });
self.discovery_task = Some(cx.spawn(async move |this, cx| {
    let data = work.await;
    let _ = this.update(cx, |this, cx| { this.discovery = data; cx.notify(); });
}));
```

- Blocking core calls (Store, git, file reads, CLI probes) go inside
  `background_executor().spawn`. The UI side never waits on them synchronously.
- `cx.spawn` gives a `WeakEntity`; `this.update(..)` fails once the view is gone, so ignore
  that error (`let _ =`) rather than unwrap.
- Keep the `Task` in an `Option<Task<()>>` field when a newer request should cancel the older
  one (replacing the field drops the old task); `.detach()` only fire-and-forget work whose
  result is still valid whenever it lands.
- Guard against stale results with a generation counter (`Shell::save_state`'s
  `generation`, `SettingsView::management_generation`, `Conversation`'s
  `submission_generation`).

## Provider turns and processes

`conversation.rs` starts a turn by spawning `runtime::run_turn(summary, prompt, settings,
control_rx, store, cancel, tx)` on the background executor. The runner streams
`runtime::Update`s over a bounded `async_channel` (capacity 8); `runtime::publish` retries a
full channel and stops when `cancel` is set or the receiver is gone. On the UI side a
`cx.spawn` loop receives updates, waits for `pulse::beat(cx)` so streamed rows land on the
shared 30 Hz frame, and applies them to the transcript only if `submission_generation` still
matches. Stop/approval controls go back through a `std::sync::mpsc` `Control` channel.

Child processes are owned by core: quitting calls `monos::begin_quit` and then
`monocode_core::process::shutdown_all()` on the background executor (`main.rs`
`stop_processes`), and `process::guard` kills CLIs left by a crashed run. Do not spawn
long-lived processes directly from a view; go through the core service that owns them.

## Errors shown to the user

Core returns `Result<_, String>`. The GPUI side stores a display string on the entity
(`self.error = Some(..)` in `shell/actions.rs` `run_action`, `ActionDialog::error`) and calls
`cx.notify()`; startup failures print with `eprintln!` and exit (`main.rs`). Write the message
for a user ("Actions are not available on remote projects."), in English.
