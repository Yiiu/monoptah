# Directory Structure

## Layout

```
Cargo.toml                    root workspace: src-tauri + crates/mobile-link (builds into target/)
src-tauri/
  Cargo.toml                  crate `monocode`, lib `monocode_lib` (names are load-bearing, see below)
  build.rs                    tauri_build + icon rerun triggers + windows-gnu symbol hiding
  tauri*.conf.json            base config + platform/dev/stable/updater overlays
  capabilities/default.json   the only capability file (all windows, "windows": ["*"])
  src/main.rs                 argv dispatch: ssh-askpass, `control`, `app`, `tui-hook`, else run()
  src/lib.rs                  module list, plugins, managed state, setup, invoke_handler, RunEvent loop
  src/<feature>.rs            one flat module per feature (~85 files)
  src/<feature>/              submodules of a large feature (checkpoint, contributors, mono_chat,
                              quick_composer, telegram)
  src/remote_bootstrap.{sh,ps1}  scripts embedded by remote.rs / remote_ssh.rs
  tests/fixtures/             data files read by unit tests (e.g. mail/ses-hello.eml)
  macos/, icons/              bundle resources; also read by crates/monocode-gpui
crates/mobile-link/src/       lib.rs (Identity, Failure, PORT, VERSION), transport.rs, duplex.rs,
                              tls.rs, transcript.rs, ffi.rs (JSON C ABI for the mobile module)
```

There is no `services/` or `commands/` split: each feature module holds its Tauri commands,
its domain logic, its persistence and its `#[cfg(test)] mod tests` together. Examples:
`clickup.rs` (commands + HTTP client + `clickup-config.json` storage + tests),
`pty.rs` (commands + `PtyHost` state + unix/windows spawn + tests).

Submodule style is mixed and both forms exist: `checkpoint.rs` + `checkpoint/turn.rs`,
`contributors.rs` + `contributors/{gh,jobs,store,...}.rs`, but `telegram/mod.rs`. Follow
whatever the feature already uses. A big feature splits "Tauri surface" from "runtime":
`telegram/mod.rs` says "`poller.rs` holds the runtime, this file the Tauri surface".

## Feature map (where things already live)

| Area | Modules |
|------|---------|
| Agent processes | `harness.rs` (spawn/kill/write/SSE, binary resolution, GUI env), `harness_updates.rs`, `background_work.rs`, `resources.rs`, `workflow_command.rs` |
| Terminals | `pty.rs`, `tui_hook.rs` (busy/idle from Claude Code hooks) |
| Persistence | `session_store.rs` (SQLite `monocode.db`), `mono_transcript.rs`, `tool_detail.rs`, `notes.rs`, `artifacts.rs`, `mono.rs` |
| Git / files | `fs.rs` (file tree, git, diffs; the largest module), `worktrees.rs`, `worktree_lifecycle.rs`, `checkpoint*`, `search.rs`, `contributors*` |
| Providers / accounts | `account_identity.rs`, `provider_session_cleanup.rs`, `provider_session_transfer.rs`, `rate_limits.rs`, `pi_usage.rs`, `devin_*.rs`, `codex_mono_store.rs`, `cursor_store.rs`, `skills.rs`, `mcp.rs` |
| Integrations | `clickup.rs`, `jira.rs`, `linear.rs`, `gitlab.rs`, `azure_devops.rs`, `telegram/`, `inbox_media.rs`, `link_preview.rs` |
| Mail | `mail.rs` (commands), `mail_store.rs`, `mail_sync.rs`, `mail_send.rs`, `mail_mime.rs`, `mail_build.rs`, `mail_thread.rs`, `mail_secrets.rs`, `aws_s3.rs`, `aws_ses.rs`, `aws_sigv4.rs` |
| Remote / mobile | `remote.rs`, `remote_ssh.rs`, `ssh_askpass.rs`, `mobile_access.rs`, `mobile_router.rs` |
| Automation / control | `control.rs`, `control_cli.rs`, `automations.rs`, `reminders.rs` |
| Windows and shell | `window.rs`, `window_transfer.rs`, `menu.rs`, `quit_hotkey.rs`, `notifications.rs`, `pasteboard.rs`, `external_editor.rs` |
| Platform-only | `macos.rs`, `macos_background.rs`, `trackpad_zoom.rs`, `quick_composer/`, `mono_chat/` (macOS); `windows.rs`, `tray.rs` (Windows) |

Shared helpers that new code must reuse rather than re-implement:

- `crate::dirs_home()`, `crate::passwd_identity()`, `crate::hide_window_console()` — `lib.rs`.
- `fs::expand_home(&str) -> PathBuf` and `fs::path_to_js(&Path) -> String` (forward
  slashes on Windows) — `fs.rs`. Paths returned to JS go through `path_to_js`.
- `harness::resolve_gui_binary`, `apply_gui_env`, `gui_search_path` — finding and running
  CLIs from a Finder/Explorer launch.
- `session_store::validate_id(value, label)` and `now_millis()`.
- `clickup::write_secret_file` — 0600 file write for tokens (used by jira, linear, gitlab,
  azure_devops, contributors/assessor).

## Adding a module or command

1. Put the command in the feature's module; create `src/<feature>.rs` only for a new
   feature. Add `mod <feature>;` to the alphabetical list at the top of `lib.rs`, with
   `#[cfg(target_os = "...")]` if it is platform-only.
2. Register every command in the single `tauri::generate_handler![...]` in `lib.rs`
   (`run()`), written as `module::command_name`. Unregistered commands fail at runtime
   from JS, not at compile time.
3. Long-lived state: create it in `run()` with `.manage(Type::new())` (e.g.
   `harness::HarnessHost`, `pty::PtyHost`, `remote::RemoteConnections`) or, when it needs
   the `AppHandle` or a data dir, from a `<module>::init(app.handle())` call inside
   `.setup(...)` that calls `app.manage(...)` (e.g. `session_store::init`,
   `control::init`, `mobile_access::init`). An `init` that must not stop startup returns
   `()` (`reminders::init`, `telegram::init`, `mail::init`); one that may returns
   `Result<(), String>` and is `?`-ed, which aborts launch.
4. Children that must die with the app are killed in `reap_harness_children` in `lib.rs`
   (`HarnessHost::kill_all`, `PtyHost::kill_all`,
   `workflow_command::cancel_all_workflow_commands`). A new long-lived child host joins it.
5. Per-module data files live under `app.path().app_data_dir()` (see
   `clickup::config_path`, `session_store::init`). That directory is keyed on the bundle
   identifier, which is why the dev identifier isolates data.

## main.rs subcommands

The same binary is also a CLI. `main.rs` checks argv before `monocode_lib::run()`:
`ssh_askpass::maybe_run()`, then `control` / `app` (`control_cli::run`, `run_app`) and
`tui-hook` (`tui_hook::run`). These modules are `pub mod` in `lib.rs` for that reason.
These entry points run without a Tauri app: `control_cli` connects to the running app
over TCP (`TcpStream::connect_timeout`) using the `MONOCODE_CONTROL_ENDPOINT` /
`MONOCODE_CONTROL_TOKEN` (or `MONOCODE_APP_*`) env that `control::configure_child` puts on
spawned agents. Keep their code paths free of `AppHandle`/`State`.

## Names that must not change

From CLAUDE.md "Never rename these": crate `monocode`, lib `monocode_lib`, npm package
`monocode-desktop` (matched by `scripts/bump-version.mjs` and used for artifact names).
Also keep `monocode.db` (`session_store::init_with`), the `MONOCODE_*` env vars and the
`monocode*` event names (`commands-and-events.md`): existing installs and the TS side
depend on them. The fork renames only user-visible text.

## mobile-link

`crates/mobile-link` has no Tauri dependency and returns `LinkResult<T> = Result<T,
String>` or a `Failure { error, code }`. Its wire constants `PORT = 38416` and
`VERSION = 1` are a protocol shared with the phone; changing them strands paired devices.
`ffi.rs` exposes a JSON `Call` enum (`#[serde(tag = "operation", rename_all =
"camelCase")]`) to the Expo module, so its field names are a contract with
`mobile/modules/monocode-link`. `crates/monocode-core/Cargo.toml` also depends on it, so a
change here needs the native checks too (`npm run native:check`, see CLAUDE.md).
