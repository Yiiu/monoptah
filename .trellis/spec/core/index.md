# monocode-core Guidelines

> Specs for `crates/monocode-core`, the UI-free Rust domain crate of Monoptah Native.
> Read this file first, then the topic files linked below that match your change.

---

## What this crate owns

`crates/monocode-core` holds every piece of Native logic that does not draw pixels:

- Provider transports and their child processes (`src/provider.rs`, `src/provider/*`,
  `src/process.rs`, `src/process/guard.rs`).
- Session storage over the installed desktop app's SQLite database (`src/store.rs`,
  `src/store/shared.rs`, `src/store/native_chunks/`).
- Accounts and credentials (`src/accounts/`, `src/docker_accounts.rs`, `src/git_credentials.rs`).
- Cloud sandboxes and cloud sessions (`src/sandboxes.rs`, `src/cloud_sessions/`,
  `src/codespaces.rs`, `src/remote/`).
- Mono workspaces and workflows (`src/mono/`, `src/mono/workflows/`).
- Git/SCM helpers (`src/git.rs`, `src/scm.rs`, `src/scm/shell.rs`, `src/checkpoint.rs`).
- Native self-update (`src/app_update/`), mobile/web link (`src/mobile/`), notes, inbox, search.

The only consumer is `crates/monocode-gpui` (`Cargo.toml` there: `monocode-core = { path =
"../monocode-core" }`, and with `features = ["test-support"]` for its dev-dependencies). The root
workspace (`Cargo.toml`, members `src-tauri`, `crates/mobile-link`) **excludes** this crate, so the
Tauri app does not link it even though `src/lib.rs` still says "shared by the native migration and
Tauri". Many modules are ports of Tauri/TS code and must stay format-compatible with it (see
[persistence-and-compat.md](./persistence-and-compat.md)).

The crate is a member of the native workspace rooted at `crates/monocode-gpui`
(`workspace = "../monocode-gpui"` in `crates/monocode-core/Cargo.toml`), so it builds into
`crates/monocode-gpui/target`.

---

## Pre-Development Checklist

Before writing code here:

1. Read the "Developing the native app", "Never rename these" and "Verifying" sections of the
   repo-root `CLAUDE.md`. They are binding; this spec does not repeat them.
2. Find the module that already owns the concept (`src/lib.rs` lists every public module) and
   read its `//!` header. Most headers state the module's rules (e.g. `src/sandboxes.rs`:
   "Background code must only ever `inspect` / `state` / `ttl` / `list`").
3. Spawning a process? Read [processes-and-platforms.md](./processes-and-platforms.md) and reuse
   `process::configure_child` / `process::isolate` and `scm::resolve_gui_binary`.
4. Touching stored data (SQLite, `workflows.json`, desktop localStorage, update feed)? Read
   [persistence-and-compat.md](./persistence-and-compat.md) before changing any field name,
   column or byte of output.
5. Returning an error that reaches the UI? Read [error-handling.md](./error-handling.md): the
   string is shown to the user verbatim.
6. Writing tests? Read [testing.md](./testing.md): Linux-portable, fake runners, no realistic
   tokens.
7. No UI types here. If you need GPUI, the code belongs in `crates/monocode-gpui`
   (see [directory-structure.md](./directory-structure.md)).

---

## Quality Check

Never build with plain `cargo test`/`cargo build`: it uses the `dev` profile and fills a second
`target/debug`. Use the aliases in `crates/monocode-gpui/.cargo/config.toml` **from inside
`crates/monocode-gpui`** (cargo reads that file by working directory):

```bash
cd crates/monocode-gpui
cargo fmt --all
cargo nclippy -p monocode-core --all-targets -- -D warnings
cargo ntest -p monocode-core <module_path_filter>      # e.g. mono::workflows::store_tests
```

From the repo root, pass the manifest and profile explicitly, as the npm scripts do:

```bash
cargo nextest run --locked --manifest-path crates/monocode-gpui/Cargo.toml \
  -p monocode-core --cargo-profile native-dev <filter>
```

Rules (from `CLAUDE.md`, "Developing the native app"):

- **Test in one pass per task.** Make all edits first, then fmt, clippy and only the tests of the
  touched modules, once. Rerun only what failed. Do not test after each small edit.
- `npm run native:check` (fmt, clippy `-D warnings`, nextest for core and gpui) is a per-batch
  gate, not a per-task one. CI (`.github/workflows/native.yml`) runs core clippy and nextest on
  Linux and Windows for every PR.
- Some core tests scan `crates/monocode-gpui/src` sources (e.g.
  `cloud_sessions/kit_prefs_tests.rs::cloud_sources_never_read_the_keychain_or_claude_credentials`),
  so a GPUI-only change can still fail a core test. Run the core filter for the area you touched.
- `src/` strings are scanned by `scripts/check-fork-branding.sh` (it lists
  `crates/monocode-core/src/`). User-visible copy says "Monoptah"; see `src/brand.rs`.
- Read exit status directly, never through a pipe (`CLAUDE.md`, "Verifying").

---

## Guidelines Index

| File | Covers |
|------|--------|
| [directory-structure.md](./directory-structure.md) | Module layout, where new logic goes, test file placement, the GPUI boundary |
| [error-handling.md](./error-handling.md) | `Result<_, String>` as the norm, typed errors with `Display`, redaction, `eprintln!` logging |
| [persistence-and-compat.md](./persistence-and-compat.md) | SQLite schema rules, desktop DB compatibility, serde conventions, atomic files, byte-exact fixtures, identifiers that never change |
| [processes-and-platforms.md](./processes-and-platforms.md) | Spawning children, the `sbx` runner, provider isolation and shutdown, GUI PATH, Windows/macOS/Linux differences |
| [testing.md](./testing.md) | Test placement, fakes and injection, temp dirs, platform gating, opt-in e2e tests, fake secrets |

Shared, cross-layer guides live in `.trellis/spec/guides/`.
