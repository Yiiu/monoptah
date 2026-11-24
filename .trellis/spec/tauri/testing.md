# Testing

All Rust tests for this layer are unit tests inside the source files. There is no
`src-tauri/tests/*.rs` integration test and no Tauri mock runtime (`tauri::test`,
`MockRuntime` are not used anywhere). `src-tauri/tests/` holds only fixture data.
No CI job runs these tests (the Tauri `ci.yml` is paused), so the local run is the gate.

## Running

```bash
cargo test -p monocode session_store          # filter by module path or test name
cargo test -p monocode-mobile-link
npm run check:rust > rust.log 2>&1; echo $?    # fmt + clippy -D warnings + all tests
```

Make all edits first, then run the filtered tests once; rerun only what failed. Read the
exit status directly, never through a pipe (CLAUDE.md "Verifying").

## Layout

- `#[cfg(test)] mod tests { use super::*; ... }` at the bottom of the module (88 files
  have one; e.g. `clickup.rs`, `session_store.rs`, `pty.rs`, `lib.rs`).
- A large test suite moves to a sibling file: `checkpoint/turn.rs` declares `mod tests;`
  and the tests live in `checkpoint/turn/tests.rs`.
- Test names are sentences describing behaviour, not `test_foo`:
  `maps_401_to_an_invalid_token_message` (`clickup.rs`),
  `a_staged_file_goes_where_it_is_told_and_holds_the_bytes` (`mail_send.rs`),
  `last_window_close_requests_quit_without_dock` (`lib.rs`).

## Testing without an AppHandle

Commands take `AppHandle`/`State`, which tests cannot build. The code is shaped so the
logic is reachable without them:

- **Pure functions extracted from commands.** `lib.rs` tests `should_request_quit(code)`
  rather than the `RunEvent` loop; `pty.rs` tests `spawn_plan` and `is_allowed_launch`.
  New command logic goes in a plain function the command calls.
- **In-memory stores.** `SessionStore::open_in_memory()` (`#[cfg(test)]`) runs the real
  `migrate`. Migration tests open a file DB and pre-record versions
  (`migrate_creates_workspace_tables_when_versions_already_recorded`).
- **Injected startup.** `session_store::init_with(data_dir, open, manage)` takes closures so
  a test covers "the complete startup path" without Tauri.
- **Trait seams for external effects.**
  `contributors::gh::GhRunner` (real `SystemGh`, test `FakeGh` with canned output);
  `telegram::poller::EventSink` (test `FakeSink` records `emit_to` calls instead of a
  window). Telegram also takes a `base` URL so tests point it at a local
  `TcpListener::bind("127.0.0.1:0")` server (`telegram/api.rs`, `poller.rs`).
- **No sleeping on clocks.** `telegram::poller::Bridge` has a `#[cfg(test)] skew_ms`
  "so rate-limit waits need no sleeping"; `GhRunner::retry_delay` is overridable.
- **Test-only hooks** are `#[cfg(test)]` methods on the real type, e.g.
  `SessionStore::conn_is_free()` to prove a long operation does not hold the lock.

## Files, processes, and fixtures

- Scratch directories: `std::env::temp_dir().join(format!("<prefix>-{}",
  uuid::Uuid::new_v4()))` (`contributors/gh.rs` `scratch_dir`, `mail_send.rs` `scratch`).
  There is no `tempfile` crate; for cleanup on panic, modules define a local
  `struct TempDir(PathBuf)` with `Drop` (`mail_secrets.rs`, `pasteboard.rs`).
- Git repos in tests are real (`git init` in a scratch dir) with network blocked:
  `git config protocol.allow never` (`contributors/gh.rs` `repo_with_origin`).
- Fixture files are embedded at compile time:
  `include_bytes!("../tests/fixtures/mail/ses-hello.eml")` in `mail_mime.rs`;
  `crates/mobile-link/tests/fixtures/legacy-identity.json` for the mobile identity format.
- Fake secrets must not look real (GitHub push protection; CLAUDE.md "No realistic
  secrets in tests"). Build them with `concat!` or use obviously fake values.

## Platform gating

- OS-specific tests: `#[cfg(all(test, windows))]` / `#[cfg(all(test, unix))]` modules
  (`lib.rs` `background_command_tests` runs `cmd.exe` through `hide_window_console`;
  `fs.rs` `preserves_unix_backslash_filenames`), or `#[cfg(target_os = ...)]` on one test
  (`lib.rs` `last_window_close_stays_alive_with_dock`, macOS only).
- Tests needing external setup are `#[ignore = "<reason>"]` and say how to provide it:
  `remote_ssh.rs` `loopback_transport_preserves_host_and_reconnects` needs
  `scripts/test-remote-ssh.py`, and reads its config from `MONOCODE_TEST_*` env vars.
- `windows.rs` tests re-run the test binary as a child (`MONOCODE_JOB_TEST_CHILD`) to check
  that job-object enrolment kills descendants; keep that pattern for process-lifetime
  tests instead of trusting mocks.

## Rules

- Never weaken an assertion to make a test pass; never keep a test that passes only
  because it asserts old "MonoCode" copy (CLAUDE.md "After every merge from upstream").
- A bug fix comes with a test in the owning module that fails without the fix.
- When a command's payload shape changes, the TS tests that mock
  `@tauri-apps/api/core` (`src/**/*.test.ts`) are part of the same change; run
  `npm run check`.
