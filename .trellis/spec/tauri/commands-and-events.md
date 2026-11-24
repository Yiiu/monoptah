# Commands and Events (IPC contract with `src/`)

The React app calls Rust through `invoke` from `@tauri-apps/api/core` and listens with
`listen` from `@tauri-apps/api/event`. There is no generated binding layer (no specta or
ts-rs in `package.json` or `src-tauri/Cargo.toml`): every TS type for a payload is written
by hand next to its caller, so **both sides are edited together**.

## Declaring a command

```rust
// src-tauri/src/session_store.rs
#[tauri::command(async)]
pub fn session_get(
    store: State<'_, SessionStore>,
    session_id: String,
) -> Result<Option<SessionRecord>, String> {
    validate_id(&session_id, "session")?;
    ...
}
```

- `pub fn` in its feature module, registered as `session_store::session_get` in the one
  `tauri::generate_handler![...]` in `lib.rs`. About 400 commands exist; names are
  `snake_case` prefixed by the feature (`pty_*`, `harness_*`, `mail_*`, `session_*`).
- Return `Result<T, String>` (393 of ~400 commands). Commands that cannot fail may return
  a bare value (`default_cwd() -> String` in `lib.rs`). See `error-handling.md` for the one
  structured error (`mail_send::SendError`).
- Injected parameters come first: `app: AppHandle`, `window: tauri::WebviewWindow`,
  `State<'_, T>` for managed state, then the JS arguments.
- Many arguments: `#[allow(clippy::too_many_arguments)]` is used rather than a wrapper
  struct when the arguments are flat (`harness::harness_spawn`). A group of related fields
  is a `Deserialize` struct argument instead (`session_upsert(session: SessionUpsert)`,
  `pty_spawn(.., launch: Option<PtyLaunch>)`).

### Sync, `(async)`, or `async fn`

A plain `#[tauri::command]` runs on the main thread and blocks the UI and every window.

- Anything that touches disk, SQLite, git, or a child process: `#[tauri::command(async)]`
  on a sync `fn` (122 commands; e.g. all `session_*`, `harness_resolve_*`, `pty_status`).
  `State` then needs the `'_` lifetime: `State<'_, PtyHost>`.
- An `async fn` command that does blocking work wraps it in
  `tauri::async_runtime::spawn_blocking(move || ...).await.map_err(|e| format!("... task
  failed: {e}"))?` (`harness::harness_write`, `harness_exec`, `claude_mcp_list`).
- Creating a window must be async: `open_new_window` in `lib.rs` notes that building it
  from a sync command deadlocks WebView2 on Windows.
- `harness_kill` is `(async)` on purpose so a kill still runs while a sibling
  `harness_write` is blocked on a wedged child's stdin (comment above it in `harness.rs`).
- Plain sync is kept only for cheap, non-blocking calls (`pty_write`, `pty_resize`,
  `pty_kill`, `set_dock_badge`).

## Argument and payload naming

- **Command arguments**: Rust parameters are `snake_case`; JS passes `camelCase` keys and
  Tauri converts them. `remote_request(machine_id, method, params)` in `remote.rs` is
  called as `invoke("remote_request", { machineId, method, params })` in
  `src/features/connections/model/connections.ts`. No command uses
  `#[tauri::command(rename_all = ...)]`.
- **Structs in and out**: `#[derive(Serialize/Deserialize)]` plus
  `#[serde(rename_all = "camelCase")]` (260 uses). Optional input fields use
  `#[serde(default)]` so older callers keep working (`pty::PtyLaunch`,
  `session_store::SessionUpsert::sidebar_hidden`).
- **Enums as strings**: `#[serde(rename_all = "lowercase")]` for simple enums
  (`artifacts::ArtifactKind`, `notifications::Permission`, `mail_store::MatchMode`).
- **Paths** returned to JS go through `fs::path_to_js` (forward slashes on Windows);
  paths from JS go through `fs::expand_home` (handles `~`, and `~\` on Windows).
- **Bytes** cross as base64 strings (`pty::emit_pty_data` encodes with
  `base64::engine::general_purpose::STANDARD`; `src/platform/tauri/pty.ts`
  `decodePtyChunk` decodes).

## Validating input from the webview

Treat every argument as untrusted. Existing guards:

- IDs used in SQL or file names: `session_store::validate_id(value, "session")`
  (ASCII alphanumeric, `-`, `_`).
- Programs to execute: `harness_spawn` and `pty_spawn` refuse anything that is not a
  resolved harness CLI (`harness::is_resolved_harness_binary`, `pty::is_allowed_launch`),
  returning `"harness_spawn: not a resolved harness CLI"`.
- Remote calls go through an allow list (`remote::supported_remote_method`).
- Secrets do not travel back to the webview: `remote.rs` says "The renderer receives
  machine metadata, never the saved bearer credential".

## Events (Rust -> webview)

Emit with `tauri::Emitter`; discard the result unless delivery matters:

```rust
// src-tauri/src/pty.rs
const DATA_EVENT: &str = "pty-data";
let _ = app.emit(DATA_EVENT, PtyData { id: id.to_string(), data });
```

- Event names are `const` strings in the emitting module (`pty.rs` `DATA_EVENT`,
  `harness.rs` `STDOUT_EVENT` = `"harness-stdout"`, `mail_sync.rs`
  `SYNC_STATE_EVENT` = `"mail://sync-state"`, `contributors/jobs.rs`
  `"contributors://sync"`, `notifications.rs` `CLICK_EVENT` =
  `"monocode:notification-click"`, `reminders.rs` `CHANGED` =
  `"monocode:reminders-changed"`, `mono_chat.rs` `"mono_chat_changed"`). Several styles
  coexist with no enforced convention; a new event follows its module's existing style.
  Existing names are a contract with `src/` and are not renamed to "monoptah".
- Payloads are a `#[derive(Serialize, Clone)]` `camelCase` struct carrying the owning id
  (`PtyData { id, data }`, `PtyExit { id, code }`), or `()` for "changed, refetch"
  signals (`session_store.rs` emits `crate::reminders::CHANGED` with `()`).
- `app.emit` broadcasts to **every** window. The TS side must filter by id; see
  `openedPtys` in `src/platform/tauri/pty.ts`, which skips other windows' terminals.
- Target one window with `app.emit_to(label, ...)`. `menu::emit_to_focused` picks the
  focused, then visible, then first workspace window. `control.rs` uses
  `emit_to(grant.window, "monocode-control-request", ...)` and treats an `Err` as
  "executor is unavailable".
- High-rate streams are coalesced: `pty.rs` `PTY_COALESCE` (8 ms) / `READ_CHUNK`
  (32 KiB), because each emit is a JS eval and a flood "froze input".

## Keeping both sides in sync

When adding, renaming, or changing a command or event:

1. Find every caller: `grep -rn '"<name>"' src` (commands are invoked by string literal;
   wrappers like `src/platform/tauri/pty.ts` and `src/features/*/model/*.ts` hold them).
2. Update the hand-written TS type next to the `invoke<T>` / `listen<T>` call; field names
   must match the `camelCase` serde output exactly, including `Option` -> `T | null`.
3. Update the TS tests that mock `@tauri-apps/api/core` (`vi.mock("@tauri-apps/api/core",
   ...)` in ~145 test files) and assert the argument object.
4. Platform-only commands are registered under `#[cfg(target_os = "macos")]` in
   `generate_handler!` (`quick_composer::*`, `mono_chat::*`, `menu::keybindings_set_overrides`).
   On other OSes invoking them rejects, so the TS caller must be platform-guarded. When a
   command should exist everywhere but act only on one OS, keep it unconditional and
   no-op inside (`set_traffic_lights_visible`, `set_dock_badge` in `lib.rs`, using
   `#[allow(unused_variables)]` on the parameters).
5. A new Tauri plugin permission or core window API used from JS needs an entry in
   `src-tauri/capabilities/default.json`; app commands do not.
6. Run `npm run check` (both halves), reading its exit status directly.
