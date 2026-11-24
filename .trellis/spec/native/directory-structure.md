# Directory Structure

> Where code lives in `crates/monocode-gpui`, and what belongs in `monocode-core` instead.

## Crate layout

```
crates/monocode-gpui/
├── Cargo.toml            # own workspace (members: ., ../monocode-core), profiles, vendor patches
├── Cargo.lock            # own lockfile; build.rs + dependency_guard.rs reject zed-scap in it
├── .cargo/config.toml    # nbuild/ncheck/nclippy/ntest/nrun aliases (native-dev profile)
├── .config/nextest.toml  # nextest profiles used by `npm run native:test` and CI
├── assets/               # SVG icons (assets/hugeicons/*), provider logos, PNGs
├── src/
│   ├── main.rs           # CLI flags, data dir, Store open, App::run, global installs, window
│   ├── shell.rs          # `Shell`: the window root entity, ~15k lines
│   ├── shell/            # `impl Shell` split by feature + shell test modules
│   ├── settings/         # `SettingsView`, `Preferences`, pages/, widgets.rs, keymap.rs
│   ├── modal.rs          # the shared modal card, scrim, barrier, buttons, prompt builder
│   ├── popover.rs        # anchored popovers, `Menu`, `Choices` dropdown
│   ├── theme.rs          # color tokens, sizes, icon and icon-button helpers
│   ├── assets.rs         # `AssetSource`: maps icon names to embedded SVGs
│   ├── runtime.rs        # `Database` type, `locked()`, provider turn runner
│   ├── workspace.rs      # serializable `Workspace` (tabs, panes, projects, preferences)
│   ├── ui_store_saves.rs # off-thread "latest value wins" setting saver
│   └── <feature>.rs / <feature>/   # one module per surface (see below)
└── tests/                # only a scratch file; real tests live in src (see testing.md)
```

Every top-level module is declared in `src/main.rs`; platform-only modules are gated there
(for example `#[cfg(target_os = "macos")] mod traffic_lights;`,
`#[cfg(not(target_os = "macos"))] mod app_log;`, `#[cfg(debug_assertions)] mod dev_reload;`).

## Module conventions

- **Feature module = `feature.rs` + optional `feature/` directory.** The parent file declares
  submodules (`conversation.rs` + `conversation/`, `settings/mod.rs` + `settings/pages/`).
  Both `foo.rs`+`foo/` and `foo/mod.rs` exist; follow whatever the module already uses.
- **`Shell` is extended, not grown in place.** `src/shell.rs` declares ~100 private submodules
  under `src/shell/`; each starts with `use super::*;` and adds an `impl Shell { ... }` block
  for one feature (`shell/actions.rs`: project actions and their dialog; `shell/settings.rs`:
  opening Settings and handling `SettingsEvent`). Put a new shell feature in a new
  `shell/<feature>.rs`, not in `shell.rs`.
- **Visibility is `pub(crate)` / `pub(super)`.** The binary crate has no public API. Shell
  submodules use `pub(super)` for what `Shell` calls, `pub(crate)` only for cross-module use.
- **Tests sit beside the code**: an inline `#[cfg(test)] mod tests` or a sibling
  `#[cfg(test)] mod <feature>_tests;` declared in the parent (`shell.rs` declares
  `escape_tests`, `cloud_view_tests`, ...; `settings/mod.rs` declares `layout_tests`).
- **Module docs name the React source being ported** (`settings/model.rs`: "Port of
  src/features/settings/model/settings.ts"). Keep that line when you port more behavior.

## Where new code goes

| Adding | Put it in | Wire it up |
| --- | --- | --- |
| A window-level dialog or panel owned by the shell | `src/<name>.rs` (view) + `src/shell/<name>.rs` (`impl Shell` open/close/handle events) | an `Option<Entity<..>>` field on `Shell`, rendered from `impl Render for Shell` (`shell.rs`), e.g. through `modal::window_overlay` |
| A settings row on an existing page | `src/settings/pages/<page>.rs`, using `self.row`/`toggle`/`segmented`/`select` from `settings/widgets.rs` | field on `Preferences` (`settings/preferences.rs`), changed via `SettingsView::update` |
| A new settings page | variant in `Section` (`settings/model.rs`), page file under `settings/pages/`, entry in `GROUPS` | rail and search read `Section`/`GROUPS` |
| A workspace-wide shortcut | action in the `actions!(workspace, [...])` list in `shell.rs`, binding in `shell::bind_keys` | handler on the `Shell` root (`on_action`) |
| A component-local shortcut | `actions!` + `pub(crate) fn bind(cx)` in that module (as `popover::bind`, `input::bind`) | call `bind` from the `App::run` closure in `main.rs` |
| An icon | SVG under `assets/hugeicons/` | a match arm in `Assets::load` (`assets.rs`), or a stroke body in `icon_body` |
| A process-wide service | module with `struct Shared(..); impl Global` + `install(cx)` + `get(cx)` | call `install` from `main.rs` (see `accounts.rs`, `ui_store_saves.rs`) |

## Core vs GPUI

`crates/monocode-core` is "UI-independent services shared by the native migration and Tauri"
(`lib.rs`). Its `Cargo.toml` has no GPUI dependency, and Tauri also consumes it.

Put in **core**: provider transports (`provider`, `agent`), persistence (`store`), accounts
and credentials, git/files/search, remote and SSH, sandboxes (the single `sbx` runner is
`crates/monocode-core/src/sandboxes.rs`), process control (`process::shutdown_all`,
`process::guard`), parsing and pure data models (`transcript::Row`, `surface::Surface`).

Put in **gpui**: entities, rendering, layout, focus, animation, keybindings, and the glue that
calls core from a background task and lands the result on an entity
(`runtime::run_turn` → `conversation.rs`).

Rules that follow from this split:

- No `gpui` types in core APIs. Core returns `Result<T, String>` or its own types; the GPUI
  side converts them for display.
- Core calls that block (Store, git, CLI spawns) run on `cx.background_executor()`, never in a
  render or event handler (`docs/native/migration.md`, "Rendering and responsiveness").
- When you need a test-only hook in core, put it behind core's `test-support` feature, which
  `monocode-gpui` enables only in `[dev-dependencies]`.
- Fix shared behavior in the shared component: the modal in `modal.rs`, popovers and selects
  in `popover.rs`, the `sbx` runner in core `sandboxes.rs` (`CLAUDE.md`: the first dialog
  input-barrier fix reached 2 of about 35 dialogs because it was applied per dialog).
