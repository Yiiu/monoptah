# Testing

> How GPUI tests are written in `crates/monocode-gpui`, and how to run only the ones you need.

## Where tests live

- Inline `#[cfg(test)] mod tests { use super::*; ... }` at the end of a module (`modal.rs`,
  `main.rs` `native_tests` and `persistence_safety_tests`), or a sibling file declared
  `#[cfg(test)] mod <name>_tests;` in the parent (`shell.rs` declares `escape_tests`,
  `cloud_view_tests`, `surface_tests`, ...; directories use `<dir>/tests.rs`).
- `crates/monocode-gpui/tests/` holds no real suite; this is a binary crate, so tests are unit
  tests inside `src`.
- Pure logic gets a plain `#[test]` (`main.rs` `bundle_plist_selects_the_data_mode`);
  anything touching entities, windows or the executor uses `#[gpui::test]`.

## Writing a GPUI test

```rust
#[gpui::test]
fn prompts_answer_with_keys_and_buttons_in_sheet_order(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_prompt_builder(prompt_builder));
    let window = cx.add_window(|_, _| gpui::Empty);
    // ...
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
}
```
(`modal.rs` tests)

- Open a window with `cx.add_window(..)` or `cx.update(|cx| cx.open_window(..))`, mutate and
  read through `window.update(cx, |view, window, cx| ..)` / `entity.read_with(cx, ..)`.
- Call `cx.run_until_parked()` after anything that spawns tasks, and `cx.refresh()` before
  reading layout. Time-based behavior uses `cx.executor().advance_clock(..)`.
- Turn animations off with `cx.update(|cx| cx.set_reduce_motion(true))` when a test clicks
  or measures animated surfaces (`modal.rs` barrier test; used in ~170 tests).
- Pointer and keyboard input goes through `VisualTestContext`: `simulate_click`,
  `simulate_mouse_move`, `simulate_mouse_down/up`, `simulate_event`, `simulate_keystrokes`.
  Locate targets with `visual.debug_bounds("selector")`, which needs a
  `.debug_selector(|| "selector".into())` on the element.
- Restart and quit paths: `cx.expect_restart()` (`main.rs` `quit_waits_for_latest_setting`).

## Shared fixtures

- **Shell window:** `crate::shell::tests::window(cx)` (`shell.rs`, `mod tests`) builds an
  in-memory `Store` with two sessions in two projects, installs a `UiStoreSaves` global for
  that store, opens a window rooted at `Shell`, and returns `(WindowHandle<Shell>, a, b)`.
  Sibling test files reuse it (`shell/surface_tests.rs` `fn window`).
- **Database:** `Store::open(Path::new(":memory:"))` wrapped as
  `Arc<Mutex<Store>>` (~120 call sites). `ui_store_saves::test_support::HeldStore` holds the
  lock to test drains; `Store::fail_next_save_commit_for_test` (core `test-support`) forces a
  save failure.
- **No real provider CLIs:** a test that can submit a turn creates
  `shell::surface_tests::NoClis::new()`, which points every `MONOCODE_<ID>_BINARY` at a
  nonexistent path and restores the env on drop.
- **Files:** use a unique `std::env::temp_dir().join(format!(..unique..))` per test
  (`cloud_settings/tests.rs`); nextest runs tests as parallel processes, so a fixed shared
  path collides (`.config/nextest.toml` comments).

## Portability rules (CI runs GPUI tests on Linux)

From `CLAUDE.md`, "Keep tests Linux-portable":

- Keystrokes use `secondary-` (cmd on macOS, ctrl elsewhere): `visual.simulate_keystrokes("secondary-w")`
  (`shell/pinned_tabs.rs`, `shell/classic_surface_tests.rs`). Never `cmd-...` in tests.
- Do not assume macOS paths, Keychain, WKWebView, AVPlayer or Metal. Use fake POSIX-style
  project paths like `"/work/monocode"` only as opaque strings.
- A test that truly runs only on macOS is gated, with a specific reason:
  - `#[cfg(target_os = "macos")]` on the test or module (`contributors/perf_tests.rs`,
    `particle_text/tests.rs`; `main.rs` declares `#[cfg(all(test, target_os = "macos"))] mod metal_tests;`),
  - or `#[cfg_attr(not(target_os = "macos"), ignore = "<specific reason>")]`
    (`markdown/mermaid.rs` names the exact upstream assert that fails on Linux).
  - Tests needing Unix process semantics use `#[cfg(all(test, unix))]` (`runtime.rs`
    `follow_up_tests`, `handoff_tests`, `visual_tests`).
- Hardware or manual-measurement tests use `#[ignore = "Requires a Metal device; run explicitly on macOS"]`
  (`metal_tests.rs`).
- Never weaken an assertion to make a platform pass; gate or fix instead.

## Secrets in tests

GitHub push protection blocks realistic tokens. Build fake tokens with `concat!` so the
literal never appears whole: `concat!("gh", "p_", "0123456789abcdefghij0123456789")`
(`mobile/cloud/tests.rs`), or use obviously fake values.

## Running tests

Make all edits for a task first, then run only the tests of the touched modules, once
(`CLAUDE.md`, "Test in one pass per task"). From `crates/monocode-gpui`:

```bash
cargo ntest -p monocode-gpui modal::tests -- --test-threads=1
cargo ntest -p monocode-gpui shell::escape_tests -- --test-threads=1
```

From the repo root:

```bash
cargo test --locked --manifest-path crates/monocode-gpui/Cargo.toml -p monocode-gpui \
  --profile native-dev <filter> -- --test-threads=1
```

`--test-threads=1` is the local equivalent of nextest's one-process-per-test isolation;
tests such as `NoClis` mutate process env and rely on running alone
(`docs/native/cloud-dev-environment.md`, "Checks"). `npm run native:test` runs the full
suite through cargo-nextest with `.config/nextest.toml`; use it, or `npm run native:check`,
once per batch, not per edit.

When a change touches macOS-only code or macOS-gated tests, also run those tests on a Mac and
`npm run native:check:mac`; Linux and Windows sessions cannot verify them.

### Windows gotchas

- **Toolchain:** vendored `gpui-pre` uses `std::hint::cold_path` and does not build on Rust
  1.93 (`E0658`). If `rustup`'s `stable` is older, pass a newer toolchain explicitly
  (`cargo +1.98.1 ntest ...`, or `RUSTUP_TOOLCHAIN=1.98.1 npm run native:dev -- --detach`).
- **A running Native Dev locks the exe:** plain `cargo ntest -p monocode-gpui` also builds the
  `monocode-gpui` binary for the integration tests and fails with `failed to remove file
  ...\target\native-dev\monocode-gpui.exe`. Run the unit tests only with
  `cargo ntest -p monocode-gpui --bin monocode-gpui <filter> -- --test-threads=1`, or close the
  dev app first.
- **Clippy on Windows** reports pre-existing dead-code and `unused_mut` lints in Windows-gated
  code (`terminal.rs`, `automations/editor.rs`, `today/note_composer.rs`,
  `runtime/remote/tests.rs`). CI lints on Linux only; judge your own files, not the crate's exit
  code.

### Keyboard shortcuts in tests

Key bindings are not registered by default. A test that sends a shortcut must call
`cx.update(crate::shell::bind_keys)` before building the window and focus something inside the
`Workspace` key context (e.g. `shell.focus_active(window, cx)`) before
`visual.simulate_keystrokes("secondary-,")`. Without both, the keystroke is silently ignored and
the assertion fails for the wrong reason.
