# Testing

> How `monocode-core` is tested. Commands and the one-pass rule are in [index.md](./index.md).

---

## Where tests live

Unit tests sit next to the code (see [directory-structure.md](./directory-structure.md)):
`foo/tests.rs` loaded by `#[cfg(test)] mod tests;`, per-topic `foo_tests.rs` declared in the
parent (`src/mono/workflows/mod.rs`, `src/cloud_sessions/`), or a small inline `mod tests`.

`tests/` holds only opt-in, real-world end-to-end checks that cost money or need credentials:

- `tests/sandbox_e2e.rs`: "Opt-in and COSTS money: it creates ONE cloud sandbox". Run with
  `MONOCODE_TEST_SBX=1 cargo test --test sandbox_e2e -- --ignored --nocapture`. It uses scratch
  stores "so the installed app's data is untouched" and a Drop guard deletes the sandbox.
- `tests/codespace_e2e.rs` follows the same pattern.
- `examples/provider_smoke.rs` and `examples/verify_*_update.rs` are manual probes.

Never make normal unit tests touch the network, a real cloud, a real keychain, or the
installed app's data directory.

## Test names describe behaviour

Names are full sentences in snake case stating the guaranteed behaviour, e.g.
`create_sends_no_stdin_and_no_secret_shaped_argv` (`src/sandboxes/tests.rs`),
`a_file_written_by_the_ts_store_round_trips_byte_identical` (`src/mono/workflows/store_tests.rs`).
Follow that; a reviewer should understand the contract from the test list alone.

## Fakes through the production seams

External effects are traits with a `System*` implementation; tests implement the trait:

- `src/sandboxes/tests.rs`: `struct Fake` implements `SbxRunner`, records every
  `(argv, stdin)` and pops queued replies (`ok(stdout)`, `fail(stderr)`). Tests assert on the
  exact argv, e.g. that no secret appears in it.
- `src/sandboxes/test_support.rs`: `AccountSbx`, a stateful fake modelling per-account
  visibility and ambiguous creates; shared with other modules as `pub(crate)` under
  `#[cfg(test)]`.
- `src/mono/workflows/store.rs`: `trait MonoFiles` "so tests can interleave a rival writer".
- Time is injected, not slept: `docker_accounts::Clock` (`Arc<dyn Fn() -> u64>`), and the web
  heartbeat test uses `tokio::time::pause` (`test-util` in `[dev-dependencies]`).
- Failure injection hooks are `#[cfg(test)]` functions on the module
  (`store::native_chunks::fail_next_conversion_at_for_test`).
- When the real process path must be exercised, tests write a tiny script as the binary into a
  temp dir (`fake_ssh` in `src/cloud_sessions/backend/docker_account_tests.rs`, `#[cfg(unix)]`).

Helpers that `monocode-gpui` tests need are exported under
`#[cfg(any(test, feature = "test-support"))]` (`src/catalog.rs`, `src/mobile/audit.rs`).

## Storage and files in tests

- In-memory SQLite: `Store::open(Path::new(":memory:"))` or
  `Store::open_monocode(Path::new(":memory:"))` (`src/app_access/tests.rs`,
  `src/accounts/tests.rs`). `:memory:` skips sidecar files and permission changes.
- Temporary directories: `tempfile::tempdir()` (cleans up on drop), or a uniquely named dir under
  `std::env::temp_dir()` removed at the end (`store_tests.rs::temp_root`, which uses
  `mono::new_id()`). Never use a fixed shared path: tests run in parallel processes.
- Byte-compared fixtures are loaded with `include_str!`/`include_bytes!` from the module's
  `fixtures/` dir and must have an `eol=lf` rule in `.gitattributes`
  (see [persistence-and-compat.md](./persistence-and-compat.md)). Shared builders live in
  `src/mono/workflows/fixtures.rs` (`example()`, `node(json)`, `patch_node`).

## Process-global state

CI runs core tests with cargo-nextest, one process per test
(`crates/monocode-gpui/.config/nextest.toml`: "process-global state (statics, env vars set by a
test) never leaks between tests"). Plain `cargo ntest` runs tests as threads in one process, so a
test that sets env vars can race there. When a test must change an env var, restore it in a
`Drop` guard (`src/accounts/tests.rs`, `struct Binary`), and prefer a nextest run for that module.
A test that needs a machine-wide resource gets a test group in `nextest.toml`.

## Platform gating (Linux-portable by default)

Core tests run on Linux and Windows in CI (`.github/workflows/native.yml`); macOS runs only
locally. Rules from `CLAUDE.md` "Developing the native app" apply. In this crate:

- Platform-specific helpers and tests are compiled only where they work: a whole module gated
  `#[cfg(all(test, unix))] mod tests` (`src/process/guard.rs`), or `#[cfg(unix)]` on single
  helpers and tests (`cloud_sessions/backend/docker_account_tests.rs::fake_ssh`).
- A test that compiles everywhere but cannot pass on one OS is ignored with a specific reason:

  ```rust
  #[cfg_attr(windows, ignore = "runs the codespace's POSIX sh script on a local clone; the script only accepts Linux paths")]
  ```
  (`src/cloud_sessions/startup_tests.rs`).
- Manual checks use `#[ignore = "<what it needs>"]`, e.g. `"needs node and web/node_modules"`
  (`src/mobile/web_link/live_tests.rs`), `"manual integration check: reads the installed desktop
  profile"` (`src/desktop_preferences.rs`).
- WebView2 LevelDB fixtures are exercised on all OSes: `rusty-leveldb` is a non-Windows
  dev-dependency for exactly that (`Cargo.toml`).
- Never weaken an assertion to make a platform pass.

## No realistic secrets

GitHub push protection blocks realistic tokens in sources. Build fake tokens with `concat!` so
the literal never appears whole:

```rust
const PAT: &str = concat!("dckr_pat_", "exampleTokenValue1234567890");  // src/docker_accounts/tests.rs
```

Use obviously fake values otherwise, and assert that secrets stay out of argv, errors and logs
(`create_sends_no_stdin_and_no_secret_shaped_argv`; `sandboxes::redact` tests).

## Source-scanning guard tests

Some tests read source files and fail on a forbidden pattern, including files in
`crates/monocode-gpui/src`: `cloud_sessions/kit_prefs_tests.rs::
cloud_sources_never_read_the_keychain_or_claude_credentials` bans keychain/credential reads and
process spawns in the cloud kit files. If one fails, fix the code; extend its allow list only for
a deliberate, commented exception.
