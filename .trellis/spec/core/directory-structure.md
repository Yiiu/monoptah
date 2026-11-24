# Directory Structure

> How `crates/monocode-core` is laid out and where new code goes.

---

## Layout

```
crates/monocode-core/
├── Cargo.toml          # member of the native workspace (workspace = "../monocode-gpui")
├── build.rs            # hashes native sources into a build identity for the probes
├── src/
│   ├── lib.rs          # one `pub mod` per domain; the full module list
│   ├── <domain>.rs     # module root (2018 style), e.g. sandboxes.rs, provider.rs, store.rs
│   ├── <domain>/       # its submodules, e.g. provider/acp.rs, store/notes.rs
│   ├── bin/web_link_headless.rs   # headless web-link host binary
│   └── */fixtures/     # test data next to the module that uses it
├── examples/           # opt-in manual probes (provider_smoke.rs, verify_native_update.rs, ...)
└── tests/              # opt-in, paid/networked e2e (sandbox_e2e.rs, codespace_e2e.rs)
```

Module roots are `foo.rs` + `foo/` (e.g. `src/sandboxes.rs` with `src/sandboxes/account.rs`).
A few directories use `mod.rs` instead (`src/app_update/mod.rs`, `src/mono/workflows/mod.rs`,
`src/store/native_chunks/mod.rs`, `src/telegram/mod.rs`). Follow whichever style the parent
already uses; do not convert one into the other.

## Main domains (see `src/lib.rs`)

| Area | Files |
|------|-------|
| Provider CLIs (Claude, Codex, OpenCode, Devin, Pi, ACP) | `src/provider.rs`, `src/provider/*`, `src/agent.rs`, `src/catalog.rs` |
| Child process policy | `src/process.rs`, `src/process/guard.rs` |
| Session storage | `src/store.rs`, `src/store/shared.rs` (desktop schema adapter), `src/store/native_chunks/` |
| Transcripts and tools | `src/transcript.rs`, `src/tool.rs`, `src/response.rs` |
| Accounts / secrets | `src/accounts/` (sealed credentials), `src/docker_accounts.rs`, `src/git_credentials.rs`, `src/credential_guard.rs` (crate-private) |
| Cloud | `src/sandboxes.rs` (`sbx`), `src/codespaces.rs` (`gh`), `src/cloud_sessions/`, `src/remote/` |
| Mono and workflows | `src/mono.rs`, `src/mono/`, `src/mono/workflows/` |
| Git / SCM | `src/git.rs`, `src/scm.rs`, `src/scm/shell.rs`, `src/checkpoint.rs`, `src/session_worktree.rs` |
| Desktop app interop | `src/desktop_preferences.rs` (reads WebKit/WebView2 localStorage), `src/store/monocode_import.rs` |
| Self-update | `src/app_update/` |
| Mobile / web link | `src/mobile/`, `src/bin/web_link_headless.rs` |
| Identity copy | `src/brand.rs` |

## The boundary with monocode-gpui

- No GPUI, no rendering, no `Window`/`Context` types here. `crates/monocode-core/Cargo.toml` has
  no gpui dependency; keep it that way.
- Core *does* own pure view-models when the logic is non-trivial and testable without a window:
  `src/mono/workflows/board_model.rs` ("no UI types, so the GPUI board and run graph draw from
  these structs"), `src/mono/workflows/run_graph_model.rs`, `src/cloud_sessions/move_ui.rs`
  ("Read-only helpers for the 'Continue in cloud' dialog"), `src/surface.rs`. Put decision
  logic, validation and data shaping here; put layout, focus, actions and drawing in GPUI.
- Core APIs are mostly **blocking** functions returning `Result<T, String>`. `src/provider.rs`
  states the rule: "Blocking work belongs to a worker; never call `run` from a render/input
  handler." GPUI calls them through `cx.background_executor().spawn(...)` and stores the error
  string in its view state (e.g. `crates/monocode-gpui/src/automations.rs`,
  `crates/monocode-gpui/src/accounts.rs`: `Err(error) => this.error = Some(error)`).
- Async/tokio is used only inside subsystems that need it (`src/mobile/`, web link, QUIC). Events
  from long-lived services go out over `async_channel` (`src/mobile/host.rs`,
  `MobileHost::open` returns an `async_channel::Receiver<Event>`).
- Test helpers GPUI needs are gated `#[cfg(any(test, feature = "test-support"))]`
  (`src/catalog.rs`, `src/mobile/audit.rs`). The `test-support` feature is never enabled in a
  release build (`Cargo.toml` comment).

## Where new code goes

- A new concept of an existing domain: a submodule under that domain's directory, declared from
  its root file. Keep the root's `//!` header accurate; it is the module's spec.
- A brand-new domain: `src/<name>.rs` plus `pub mod <name>;` in `src/lib.rs`. Use
  `pub(crate)` for anything GPUI does not need (as `credential_guard` is).
- Seams for external effects go in the module as traits with a `System*` real implementation
  (`SbxRunner`/`SystemSbx` in `src/sandboxes.rs`, `SystemGh` in `src/codespaces.rs`,
  `MonoFiles` in `src/mono/workflows/store.rs`). Constructors that inject seams sit beside the
  `system()` one (`CloudSessions::new` vs `CloudSessions::system`, see `src/cloud_sessions.rs`).
- Copy shown to users, product names and bundle ids come from `src/brand.rs`
  (`PRODUCT`, `NATIVE_PRODUCT`, `DESKTOP_APP_ID`, `NATIVE_APP_ID`).
- Ports of TS/Tauri code keep a pointer to their origin in the header
  (`src/process.rs`: "extracted from src-tauri/src/background_work.rs";
  `src/mono/workflows/board_model.rs`: "Port of `boardModel.ts`").

## Test file placement

Both forms exist; match the neighbours:

- Sibling file declared from the root: `#[cfg(test)] mod tests;` at the end of `src/sandboxes.rs`
  loads `src/sandboxes/tests.rs`.
- Per-topic `_tests.rs` files declared in the parent: `src/mono/workflows/mod.rs` has
  `#[cfg(test)] mod store_tests;`, `mod yaml_tests;`, ...; `src/cloud_sessions/` has
  `kit_tests.rs`, `startup_tests.rs`, etc.
- Small inline `#[cfg(test)] mod tests { use super::*; ... }` (e.g. `src/git.rs`).

Shared test builders live in a `#[cfg(test)]` module (`src/mono/workflows/fixtures.rs`,
`src/sandboxes/test_support.rs`). See [testing.md](./testing.md).
