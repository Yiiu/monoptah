# Tauri Backend (src-tauri + crates/mobile-link)

> Rules for the Rust side of the Tauri desktop app. Load this before implementing or
> reviewing anything under `src-tauri/` or `crates/mobile-link/`.

## What this layer owns

- `src-tauri/` — crate `monocode`, lib `monocode_lib` (`src-tauri/Cargo.toml`). It is the
  host for the React webview in `src/`: it spawns and supervises agent CLIs
  (`harness.rs`) and terminals (`pty.rs`), keeps the SQLite session store
  (`session_store.rs`), syncs and sends mail, talks to issue trackers (ClickUp, Jira,
  Linear, GitLab, Azure DevOps), runs the control CLI (`control.rs`, `control_cli.rs`), and
  owns the native window, menu, tray and macOS integrations.
- `crates/mobile-link/` — crate `monocode-mobile-link`: the QUIC + mutual-TLS link to the
  phone. Used by `src-tauri/src/mobile_access.rs` and `mobile_router.rs`, **and** by
  `crates/monocode-core` (native app) and the Expo module in `mobile/modules/monocode-link`.
- Both share the root workspace in `Cargo.toml` (`members = ["src-tauri",
  "crates/mobile-link"]`) and build into `target/`. The GPUI workspace
  (`crates/monocode-gpui`, `crates/monocode-core`) is excluded and is **not** this layer.

Status: releases are native-only. The Tauri CI and release workflows
(`.github/workflows/ci.yml`, `release.yml`) are `workflow_dispatch` only, so **no CI job
runs the Tauri Rust tests** on a PR. `native.yml` lists `src-tauri/**` in its path filter
only because native code reads files from here (see `platform-and-config.md`). Local
checks are the only gate for this layer.

## Pre-Development Checklist

1. Read `directory-structure.md` to find the module that already owns the feature; most
   features already have one (e.g. `mail*.rs`, `clickup.rs`, `worktrees.rs`).
2. Adding or changing a command or event: read `commands-and-events.md` and find the TS
   caller (`grep -rn "\"<command_name>\"" src`). Both sides change in the same commit.
3. Spawning a process: use the helpers in `platform-and-config.md`
   (`harness::apply_gui_env`, `harness::spawn_managed`, `hide_window_console`).
4. Touching `tauri.conf.json`, capabilities, identifiers, or anything named `monocode*`:
   read `platform-and-config.md` and the CLAUDE.md sections "Never rename these" and
   "After every merge from upstream".
5. If `crates/mobile-link` changes, the native app and the mobile module are affected
   too; check `crates/monocode-core` callers.
6. Code ported into `crates/monocode-core` mirrors some files here (e.g.
   `crates/monocode-core/src/attachment.rs` mirrors `src-tauri/src/fs.rs`,
   `checkpoint.rs` mirrors `src-tauri/src/checkpoint/turn.rs`). A behaviour fix may need
   to land in both.

## Quality Check

Run from the repo root. Read each command's exit status directly, never through a pipe
(`cmd | tail` reports `tail`'s status). Do not rerun tests after every small edit: make
all edits, then run once and rerun only what failed.

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p monocode <filter>             # e.g. session_store, pty::tests
cargo test -p monocode-mobile-link <filter>
npm run check:rust > rust.log 2>&1; echo $?  # fmt + clippy + full cargo test
```

`npm run check:rust` is defined in `package.json` as
`cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test`.
`npm run check` adds vitest and `tsc --noEmit`; run it when a command/event contract
changed, since the TS side is the other half of the contract.

Platform note: clippy and tests only compile the current OS's `#[cfg]` branches. A change
to `macos.rs`, `windows.rs`, `tray.rs` or a `#[cfg(target_os = ...)]` block cannot be
checked from another OS; say so in the review rather than claiming it was verified.

To see a change live use `npm run tauri:dev` (dev identifier), never plain
`npm run tauri dev`. See CLAUDE.md "Seeing a change live".

## Files

| File | Covers |
|------|--------|
| [directory-structure.md](./directory-structure.md) | Module layout, `lib.rs` wiring, where new commands/services go, mobile-link's role |
| [commands-and-events.md](./commands-and-events.md) | IPC contract with `src/`: command declaration and registration, serde naming, events |
| [error-handling.md](./error-handling.md) | `Result<T, String>`, structured error exceptions, lock poisoning, logging |
| [platform-and-config.md](./platform-and-config.md) | Process spawning, Windows job object, macOS-only modules, tauri config files and fork invariants |
| [testing.md](./testing.md) | Unit-test layout, injection seams, scratch dirs, fixtures, platform-gated tests |

Cross-layer thinking (frontend <-> backend data flow) is in
[`../guides/cross-layer-thinking-guide.md`](../guides/cross-layer-thinking-guide.md).
