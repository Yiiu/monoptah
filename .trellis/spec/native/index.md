# Native App (GPUI) Guidelines

> How the native desktop app in `crates/monocode-gpui` is built, and what to check before
> changing it. Read this file first, then the topic files the checklist points to.

## What this layer owns

`crates/monocode-gpui` is Monoptah Native: the GPUI (`gpui-pre` 0.3.9, vendored under
`vendor/gpui-pre*`) window, shell, composer, transcript, editor, terminal dock, settings and
platform chrome. It is a binary crate (`src/main.rs`), not a library.

- It has its **own Cargo workspace** (`crates/monocode-gpui/Cargo.toml`, `[workspace]` with
  members `.` and `../monocode-core`) and its own `Cargo.lock`. It builds into
  `crates/monocode-gpui/target`, not the repo-root `target`.
- `monocode-core` holds every UI-independent service (providers, `Store`, accounts, git,
  remote, sandboxes). It has no GPUI or Tauri dependency (`crates/monocode-core/src/lib.rs`:
  "UI-independent services"; `docs/native/migration.md`, "Code layout").
- The production React/Tauri app (`src/`, `src-tauri/`) is the behavior reference. Many
  modules say which React file they port (for example `popover.rs` ports
  `src/shared/ui/Popover.tsx`, `settings/mod.rs` ports `SettingsView.tsx`).

## Pre-Development Checklist

| Kind of change | Read |
| --- | --- |
| Any change in this crate | this file, then [directory-structure.md](./directory-structure.md) |
| New view, dialog, menu, dropdown, icon, color, keybinding, focus behavior | [component-guidelines.md](./component-guidelines.md) |
| New state, preference, persisted value, global, background work, provider or process call | [state-and-async.md](./state-and-async.md) |
| Any new or changed test, or a macOS-only code path | [testing.md](./testing.md) |
| Logic that is not drawing (parsing, storage, process control) | [directory-structure.md](./directory-structure.md), "Core vs GPUI" |

Also read the repo-wide rules in `CLAUDE.md`, sections "Developing the native app",
"Seeing a change live" and "Verifying". They are summarized below, not repeated in full.

## Quality Check

The native workspace is always built with the `native-dev` profile. Plain `cargo test`,
`build`, `check` or `clippy` uses `dev` and fills a separate multi-GB `target/debug`.

From inside `crates/monocode-gpui` use the aliases in
`crates/monocode-gpui/.cargo/config.toml`:

```bash
cargo nbuild    # build --locked --profile native-dev
cargo ncheck    # check --locked --profile native-dev
cargo nclippy   # clippy --locked --profile native-dev
cargo ntest     # test --locked --profile native-dev
cargo nrun      # run --locked --profile native-dev
```

There is no `nfmt` alias; formatting has no profile. From the repo root, pass
`--manifest-path crates/monocode-gpui/Cargo.toml --profile native-dev` as the npm scripts do:

```bash
M=crates/monocode-gpui/Cargo.toml
cargo fmt --manifest-path $M --all
cargo clippy --locked --manifest-path $M -p monocode-core -p monocode-gpui --all-targets --profile native-dev -- -D warnings
cargo test --locked --manifest-path $M -p monocode-gpui --profile native-dev <filter> -- --test-threads=1
```

Rules (from `CLAUDE.md`, "Developing the native app"):

- **Test in one pass per task.** Make every edit first, then run fmt, clippy and only the
  tests of the touched modules, once. Rerun only what failed. Each run relinks the large
  `monocode-gpui` test binary (median 1.4 min, p90 4.1 min).
- **Full gate once per batch:** `npm run native:check` (script tests, fmt, strict clippy,
  `npm run native:test` via cargo-nextest) before a release or merge, not per task. CI
  (`native.yml`) runs GPUI fmt, clippy and tests on Linux and `cargo check --all-targets` on
  Windows for every PR.
- **macOS-only code:** also run `npm run native:check:mac` (clippy of code Linux and Windows
  never compile), and run touched macOS-gated tests on a Mac. Never report macOS-only checks
  as passing from a Linux or Windows session (`docs/native/cloud-dev-environment.md`).
- **Seed the cache in a new worktree:** copy an existing `crates/monocode-gpui/target` before
  the first cargo command (`cp -c -R` on macOS, `cp -R` elsewhere).
- Read exit codes directly, never through a pipe (`CLAUDE.md`, "Verifying").
- Do not add `split-debuginfo` to the profiles (reasons in the `[profile.native-dev]` comments
  of `crates/monocode-gpui/Cargo.toml`).

To see a change live: `npm run native:dev -- --detach` (Native Dev keeps its own data in
`Monoptah Development`, see `data_dir()` in `src/main.rs`). Remove any `.app` bundle you build
outside the release flow.

## Files in this directory

| File | Covers |
| --- | --- |
| [directory-structure.md](./directory-structure.md) | Module layout, where new views, dialogs, settings pages and actions go, core vs GPUI split |
| [component-guidelines.md](./component-guidelines.md) | Views and entities, shared modal and popover components, theme tokens, icons, actions and keybindings, focus |
| [state-and-async.md](./state-and-async.md) | Entities and globals, the `Store` handle, preference and UI state persistence, background work and provider turns |
| [testing.md](./testing.md) | `#[gpui::test]`, `TestAppContext` / `VisualTestContext`, shared fixtures, Linux portability, macOS gating, fake secrets, filtered runs |
