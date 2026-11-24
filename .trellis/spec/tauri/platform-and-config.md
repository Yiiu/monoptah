# Platform Code, Process Spawning, and Tauri Config

## Spawning child processes

The app is usually launched from Finder/Explorer, so it has a bare `PATH`, may lack
`HOME`/`USER`/`SHELL`, and on Windows every console child would flash a window. Never
call `Command::new(...).spawn()` raw. Use the existing layers:

| Need | Helper |
|------|--------|
| Find a CLI the way a terminal would | `harness::resolve_gui_binary(name)` (login-shell PATH, then inherited PATH, then fixed dirs such as `~/.local/bin`, `/opt/homebrew/bin`, `%APPDATA%/npm`) |
| Frequent git calls that must not wait on a slow `.zshrc` | `harness::cached_gui_search_path()` |
| Any helper process (git, gh, CLIs) | `harness::apply_gui_env(&mut cmd)`: sets PATH, fills HOME/USER/SHELL from `passwd_identity()`, and calls `crate::hide_window_console` |
| Only hide the Windows console | `crate::hide_window_console(&mut cmd)` (`CREATE_NO_WINDOW`, `lib.rs`) |
| An agent/harness child that must die with the app | `harness::prepare_child` -> `isolate_child` -> `spawn_managed` (see below) |

`contributors/gh.rs` `SystemGh::run` is the model for a helper process: resolve with
`resolve_gui_binary`, set non-interactive env (`GIT_TERMINAL_PROMPT=0`,
`GH_PROMPT_DISABLED=1`, pagers to `cat`), `stdin(Stdio::null())`, piped output,
`apply_gui_env`, and a timeout.

Managed agent children (`harness.rs`):

- `isolate_child` lowers priority (`background_work::configure_child`, a `pre_exec`
  `setpriority` on unix that "must not allocate or lock"), stamps
  `MONOCODE_HARNESS_PARENT` with our pid, and starts a new process group
  (`process_group(0)` on unix, `CREATE_NEW_PROCESS_GROUP` on Windows).
- `spawn_managed` on Windows is `windows::spawn_managed`: spawn `CREATE_SUSPENDED`, assign
  to the app's single `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` job, then resume, so descendants
  die if the app crashes. PTYs use `windows::spawn_pty` (`spawn_command_in_job`, vendored
  `portable-pty` in `vendor/portable-pty`). The app itself stays outside the job.
- On unix, `spawn_retrying_text_file_busy` retries `ETXTBSY` on Linux;
  `harness::reap_orphaned_harness_processes()` runs at startup (unix only) to kill
  children a crashed previous run left behind, keyed on `MONOCODE_HARNESS_PARENT`.
- Every spawn in a working directory takes `worktree_lifecycle::reserve_spawn(&workdir)?`
  so a worktree cannot be removed while a process starts in it (`harness_spawn`,
  `pty_spawn`).
- Shutdown kills through `reap_harness_children` in `lib.rs`; a new kind of long-lived
  child must be added there.

`windows::initialize()` (called first in `run()`) also restricts DLL search to the app
dir and System32 before any PTY opens.

## Platform-specific modules

`#[cfg]` is on the `mod` line in `lib.rs`:

- macOS only: `macos.rs` (traffic lights, blur, dock badge and menu, dev bundle),
  `macos_background.rs`, `trackpad_zoom.rs`, `quick_composer/`, `mono_chat/`. Uses
  `objc2*` crates from the macOS-only `[target.'cfg(target_os = "macos")'.dependencies]`.
- Windows only: `windows.rs` (job object, DLL search), `tray.rs` (closing hides the window;
  the tray is "the way back"). `tauri_plugin_single_instance` is registered first in
  `run()` on Windows so a second launch reopens the hidden window.
- Linux has no module of its own; behaviour differs inline (`should_request_quit` in
  `lib.rs`: on Linux and Windows closing the last window quits, on macOS it stays in the
  dock; `/proc` process scanning in `harness.rs`).
- Non-macOS windows are undecorated: `.setup` calls `set_decorations(false)` and
  `set_shadow(true)`; `tauri.windows.conf.json` and `tauri.linux.conf.json` also set
  `"decorations": false`.
- Keyring backends differ per OS in `src-tauri/Cargo.toml` (`apple-native`,
  `windows-native`, `linux-native-sync-persistent`); secrets go through `mail_secrets.rs`,
  `mobile_access.rs`, `codex_mono_store.rs`. Integration tokens (ClickUp, Jira, Linear,
  GitLab, Azure DevOps) are JSON files in the app data dir written with
  `clickup::write_secret_file` (mode 0600 on unix).
- Paths: `dirs_home()` prefers `USERPROFILE` on Windows and `HOME` elsewhere, then
  `HOMEDRIVE`+`HOMEPATH`, then passwd. `path_to_js` turns `\` into `/` on Windows only.

When you edit a `#[cfg(target_os = ...)]` branch for an OS you are not on, it is not
compiled by your local clippy or tests. State this in the hand-off.

## Tauri config files (`src-tauri/`)

| File | Role |
|------|------|
| `tauri.conf.json` | Base config. Must keep `productName: "Monoptah"`, `identifier: "com.monoptah.desktop"`, the fork updater `pubkey` and `endpoints` (`https://github.com/hhoangg/monoptah/releases/latest/download/latest.json`), window `label: "main"` |
| `tauri.dev.conf.json` | Overlay used by `npm run tauri:dev`: `productName: "Monoptah Dev"`, `identifier: "com.monoptah.desktop.dev"` |
| `tauri.stable.conf.json` | Overlay for `npm run tauri:stable`: only changes `beforeDevCommand` to `npm run dev:stable` (keeps the production identifier) |
| `tauri.windows.conf.json`, `tauri.linux.conf.json` | Platform overlays Tauri merges automatically: undecorated window, NSIS (`installMode: currentUser`) / deb+appimage, `createUpdaterArtifacts: false`, `resources: null` |
| `tauri.updater.conf.json` | Re-enables `createUpdaterArtifacts`; the paused `release.yml` passes it to the Linux and Windows `tauri build` steps |
| `capabilities/default.json` | Plugin and core-window permissions for all windows (`"windows": ["*"]`) |

Security config in `tauri.conf.json`: a strict CSP (images from `asset:` and a few named
hosts; `frame-src` allows the `monoartifact:` scheme registered in `lib.rs` via
`register_uri_scheme_protocol(artifacts::HTML_SCHEME, ...)`); `assetProtocol.scope` is
`$APPDATA/**` only. Remote images are proxied through the backend instead of widening the
CSP (`inbox_media.rs`: "without opening `img-src` / `media-src` to GitHub's CDNs").

Fork invariants (full list in CLAUDE.md "After every merge from upstream"):

- After an upstream merge, confirm the four `tauri.conf.json` keys above and run
  `./scripts/check-fork-branding.sh` (it allow-lists `src-tauri/src/macos.rs`,
  `skills.rs`, `control_cli.rs` with reasons).
- `src-tauri/Cargo.toml` still carries upstream `description`/`repository`/`homepage`;
  the crate and lib names must stay (`scripts/bump-version.mjs`). Versions are set with
  `npm run set-version`, not by hand.
- `.github/workflows/ci.yml` and `release.yml` stay `workflow_dispatch` only.

## Dev identifier isolation

`npm run tauri:dev` passes `--config src-tauri/tauri.dev.conf.json`, so the dev app's
`app_data_dir()` (and therefore `monocode.db`, tokens, mail) is separate from the
installed app. Never run plain `npm run tauri dev`. On macOS `macos::ensure_dev_bundle`
(debug builds only) wraps the binary in a `.app` and merges `tauri.conf.json` with
`option_env!("TAURI_CONFIG")` (`TAURI_CONFIG_OVERLAY`), because "WebKit keys localStorage
and caches on CFBundleIdentifier". Do not remove that overlay merge. See CLAUDE.md
"Seeing a change live".

## Files the native app reads from here

`crates/monocode-gpui/src/monos/status.rs` embeds
`src-tauri/macos/AppIcon.icon/Assets/icon.png`, and `native.yml`'s path filter includes
`src-tauri/**`. Moving or deleting files under `src-tauri/macos` or `src-tauri/icons`
breaks the native build.
