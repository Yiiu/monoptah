# Implement: test coverage and appearance check

Branch: lane `feat/agent-overview-tree-tests`, cut from `feat/agent-overview-tree` **after** the other follow-up lanes have merged back.

1. **Hermes mapping** (core): write a test in `provider/acp` tests with a fake ACP session that dispatches background work.
2. **Notification click seen** (GPUI): in `shell/notifications` tests, simulate a click target and assert both `seen_at` and the row presentation.
3. **Classic and Focused row text** (GPUI): in `shell/rails` and `shell.rs` session-row tests, write one test per label, plus one for the shell chip.
4. **Mobile catalog** (GPUI): in `shell/mobile_host` tests, use snapshot-style assertions on the JSON fields.
5. **Mono-dispatched sessions in the tree** (GPUI): in `shell::project_tree::tests`, create a Mono-launched session and assert its tree row and its mascot.
6. **Mono plan checklist** (GPUI): in `monos::dashboard` tests, cover counts that change as the session statuses change.
7. **Appearance check**:
   - Run `RUSTUP_TOOLCHAIN=1.98.1 npm run native:dev -- --detach` and switch appearance in Settings.
   - Capture screenshots with PrintWindow; see the session notes in the parent `handoff.md` for the Windows caveats.
   - Save them under `research/appearance-check/` and add the findings at the end of `prd.md`.

Commands (one pass at the end):

```bash
cargo fmt --all
cargo +1.98.1 nclippy -p monocode-core -p monocode-gpui --all-targets -- -D warnings   # judge your files only
cargo +1.98.1 ntest -p monocode-core provider:: -- --test-threads=1
cargo +1.98.1 ntest -p monocode-gpui --bin monocode-gpui -- shell::notifications shell::rails shell::mobile_host shell::project_tree monos::dashboard --test-threads=1
```

Hand-over: a checklist mapping items 1–6 to their test names, plus the screenshot list. Merge back and push.
