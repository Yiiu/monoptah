# Processes and Platforms

> How `monocode-core` starts external programs (git, gh, sbx, provider CLIs) and handles
> macOS, Linux and Windows differences.

---

## Every child goes through `src/process.rs`

`src/process.rs` is the single child policy (header: "Child policy extracted from
src-tauri/src/background_work.rs"). Pick the right entry point; do not hand-roll flags:

| Function | Use for | What it does |
|----------|---------|--------------|
| `process::configure_child(&mut cmd)` | Short helpers: git, gh, sbx | Resets the signal mask (Unix), lowers CPU/IO priority, and on Windows sets `CREATE_NO_WINDOW` |
| `process::isolate(&mut cmd)` | Long-lived provider/sign-in CLIs | `configure_child` + own process group (`process_group(0)` / `CREATE_NEW_PROCESS_GROUP`) + `MONOCODE_HARNESS_PARENT` env |
| `process::guard::arm(&child, &cmd)` | Right after spawning an isolated child that must die with the app | Unix watchdog in the child's group plus a record file swept on next start; Windows job object with kill-on-close |
| `process::retire(child)` | Stopping a provider | Close stdin, SIGTERM the leader, grace, then SIGKILL the group, on a background thread |
| `process::shutdown_all()` | App quit | SIGTERM leaders first, then kill what is left; afterwards nothing may be parked |

Why it matters:

- **Windows console windows.** The native app is a GUI-subsystem process; "a GUI-subsystem
  parent gives every console child (git, gh) its own visible console window unless asked not to"
  (`configure_child`). A spawn that skips `configure_child`/`isolate` flashes a console on
  Windows. A few legacy sites set `creation_flags(0x0800_0000)` by hand
  (`src/checkpoint.rs::git_command`); new code calls `configure_child`.
- **Signal mask.** GUI executor threads may block signals and exec keeps the mask, so
  Tokio-based children never get SIGCHLD (`reset_signal_mask`). The `pre_exec` closures do
  "only system calls here: the post-fork child must not allocate or lock". Keep it that way.
- **OAuth token rotation.** Killing Claude Code's whole group while it saves a rotated refresh
  token signs the account out (`retire` doc comment). Stop providers through `retire`, never
  with a bare `child.kill()`.
- `guard::arm` writes argv to a record file: "Do not use it for a command whose argv carries a
  secret."

Children that belong to a session are registered (`process::track`, `owned_by`, session
epochs) so archive and quit can find them. Provider spawns call `process::begin_turn()` before
anything is spawned (`src/provider.rs`, "Before the cancel check and any spawn").

## Finding binaries and the GUI environment

A Finder-launched app inherits launchd's bare PATH. Never rely on the inherited `PATH`:

- Resolve with `crate::scm::resolve_gui_binary(name)` (login-shell PATH first, inherited PATH,
  then fixed dirs like `~/.local/bin`, `/opt/homebrew/bin`, `AppData/Roaming/npm`,
  `C:\Program Files\Git\cmd`; `src/scm/shell.rs::gui_search_path_from`).
- Apply `crate::scm::shell::apply_gui_env(&mut cmd)` so the child gets that PATH, HOME/USER/SHELL
  and a UTF-8 `LANG`. The login-shell PATH is read once with `$SHELL -lic` and cached; on Windows
  it is just `PATH` (`load_login_shell_env`). `cached_gui_search_path` never starts the shell;
  use it on paths that must not block.
- On Windows a name without extension resolves to `.exe`, `.cmd`, `.bat` or `.com`
  (`existing_binary`, and the same list in `src/provider.rs`).
- A missing binary is a normal, user-visible error, not a panic: `sandboxes::spawn_error` maps
  `ErrorKind::NotFound` to "Docker Sandboxes CLI (sbx) is not installed".

## The `sbx` runner: the pattern for CLI wrappers

`src/sandboxes.rs` is the reference implementation (`CLAUDE.md` names it as the shared runner to
fix at the root). Its rules carry over to `gh` (`codespaces::GhRunner` / `SystemGh`) and other
CLIs:

- **One injectable seam.** All calls go through `trait SbxRunner { fn run(&self, args,
  stdin) -> Result<SbxOutput, String>; fn run_as(...) }`. `SystemSbx` "is the only place an
  `sbx` process is spawned"; tests pass fakes. Add verbs as helpers that take
  `&dyn SbxRunner`, never by spawning `sbx` elsewhere.
- **Never wait for input.** Every call is `sbx --cloud --no-input ...` (`with_base_flags`), with
  `NO_COLOR=1` and `GIT_TERMINAL_PROMPT=0`; stdin is `Stdio::null()` unless data is passed.
- **Secrets travel over stdin or env, never argv.** Accounts inject `DOCKER_ACCESS_TOKEN` via
  `docker_accounts::apply_env`, applied after the GUI env "so an inherited DOCKER_ACCESS_TOKEN
  can not win over the account".
- **Bounded time.** `run_piped` reads stdout/stderr and writes stdin on separate threads (no pipe
  deadlock) and kills the child past `call_timeout` (20 s, 120 s for `create`/`rm`/`stop`), then
  returns an actionable message.
- **One retry on auth rejection** (`run_as_account`): invalidate the cached JWT, retry once, then
  report the account error. Never fall back to the machine login.
- **Billing safety.** "Background code must only ever `inspect` / `state` / `ttl` / `list`." Any
  `ssh`/`exec` into a stopped sandbox resumes it and starts billing.
- Failure text is `failure(out, what)`: stderr else stdout, passed through `redact`.

## git

`src/git.rs::git(cwd, args)` is the plain helper: it refuses remote projects
(`files::remote::is_remote`, paths starting `remote://`), sets `GIT_TERMINAL_PROMPT=0`, calls
`configure_child`, and returns trimmed stderr as the error. Read-only status helpers must never
take Git's index lock (`git::uncommitted` doc: "Never takes Git's index lock").

## Provider CLIs

`src/provider.rs` builds each provider command from `request.binary` with `apply_gui_env`, the
session's environment, the account (`settings.account.apply(kind, &mut command)?`), then
`process::isolate`. Provider-specific flags live in the `match request.kind` there and in
`provider/opencode.rs`, `provider/acp.rs`, `provider/rewind.rs`. Blocking provider work runs on a
worker thread, never in a GPUI render/input handler (`provider.rs` header).

## Platform differences you must handle

- Use `#[cfg(unix)]`, `#[cfg(windows)]`, `#[cfg(target_os = "macos")]` blocks inside one function
  and end with `let _ = command;` so the non-matching platform still compiles cleanly (pattern in
  `configure_child`, `isolate`). Every target must build: CI runs clippy and tests on Linux and
  Windows; macOS-only code is checked locally (`npm run native:check:mac`).
- Process trees: Unix signals the process group (`libc::kill(-pid, ...)`); Windows uses
  `taskkill /PID <pid> /T /F` with `CREATE_NO_WINDOW` (`process::terminate`).
- Data dirs: macOS `~/Library/Application Support`, Windows `%APPDATA%` (WebView2 data under
  `%LOCALAPPDATA%`), Linux `$XDG_DATA_HOME` or `~/.local/share`
  (`store/monocode_import.rs::monocode_database`, `credential_guard.rs`).
- File permissions (`0o600`) are Unix-only `#[cfg(unix)]` steps; Windows relies on the profile ACL.
- Keychain: `keyring` with `apple-native`, `windows-native` or `linux-native-sync-persistent`
  per target (`Cargo.toml`).
- Paths handed to JS/TS-shaped data use forward slashes on Windows
  (`scm::shell::path_to_js`); `~` is expanded with `scm::shell::expand_home`.
