# Implement: 第1期 状态准确

Two PRs, both from `main`. Each step has to build on its own, one commit per step. Following
CLAUDE.md "Developing the native app": make all the edits for a step first, then run fmt,
clippy and **only the filtered tests** once.

```bash
M=crates/monocode-gpui/Cargo.toml
cargo fmt --manifest-path $M --all
cargo clippy --locked --manifest-path $M -p monocode-core -p monocode-gpui --all-targets --profile native-dev -- -D warnings
cargo test --locked --manifest-path $M -p <crate> --profile native-dev <filter> -- --test-threads=1
```

Before the first cargo command in a new worktree, seed the build cache by copying
`crates/monocode-gpui/target` from the main checkout.

## PR-A — `fix(native): show accurate agent status across the app`

Branch: `fix/agent-status-core`

1. **Verify T1** (design §3): read `provider/follow_up.rs` and the runtime to answer: what happens
   to assistant frames that arrive after `Event::Done`? Write the answer and the chosen path
   (auto-started turn, or the status-only fallback) into `research/state-plumbing.md`. Do this
   before any code change.
2. **Core model**: add `monocode-core/src/agent_status.rs` with `Activity`, `AgentStatus`,
   `StatusInput`, `derive()`, `label()` and unit tests covering every rule in design §2: each
   precedence pair, stale at 30 m, usage-limit wait, unread versus seen, Idle. Test filter:
   `agent_status`.
3. **Claude adapter**: `TaskKind` on `BackgroundTask`; the hold-back waits only for wake tasks;
   the authoritative prune on `background_tasks_changed` and on terminal `task_updated`; the T1
   path. Update `provider/tasks.rs:972` and the `follow_up.rs:435-478` tests to the new
   semantics, and add tests for:
   - shell-only → Done, with Background still published;
   - a task vanishing from the list → no stuck pending;
   - a subagent wake → a new turn.

   Filter: `provider::`.
4. **Persistence**: the `session_activity` table, the migration that marks all existing sessions
   read, the upsert API and the interrupted-on-restart sweep. Store tests: filter `store::`.
5. **Shell status cache**: `Shell::agent_status`, `refresh_status` replacing the derivation in
   `refresh_session_mark`, `last_input_at` plus the seen rule (design §5), and removal of
   `unseen_finished`. `Update.background` now emits `SessionChanged`. Adapt
   `shell.rs:10505` and `session_filters.rs:124`, and add a test for the seen rule (window active
   but idle for more than 180 s → stays unread). Filter: `shell::`.
6. **Consumers**: switch every row in design §6 to the cache. Row text: `label()`, the shell
   chip, the `Waiting` mark state. Update the pinned tests listed in
   `research/state-plumbing.md` §7 (TUI, live agents, Monos, Inbox, mobile catalog) to the new
   semantics. Filters: the touched modules only.
7. **Self-check**: run the full `npm run native:test:dev` filters for the touched modules once,
   then `cargo nclippy`. Then do a manual run with `npm run native:dev -- --detach` and a Claude
   session that starts a background dev server and a background subagent, and check the
   sidebar in both layouts.

## PR-B — `feat(native): alert on Windows when an agent needs you`（暂缓，本期不做）

Branch: `feat/windows-attention`. It can start in parallel with PR-A.

1. **Dependencies and cfg**: add the Windows `windows` 0.62 dependency and `raw-window-handle`,
   update `Cargo.lock`, and widen the dead-code `cfg_attr`s. `cargo check` for the Windows target
   if a toolchain is available; otherwise CI's Windows `cargo check` is the gate.
2. **Portable pieces, with tests**: `Target::from_tag` on Windows, the 5 s dedupe in
   `delivery.rs`, toast title and body formatting, and the badge pixel buffer (digit glyphs,
   `9+`). Filter: `notifications::`.
3. **`platform_windows.rs`**: app identity at startup (dev vs installed), toast via GPUI,
   click routing, `SetOverlayIcon` badge from the captured HWND, `FlashWindowEx` until focus,
   `PlaySoundW` in `sound.rs`.
4. **Manual test on Windows 11** (record the results in the PR body):
   - The window in the background and a session needing input → toast, badge, flash and sound.
   - Click "Open" → that session opens and is marked seen.
   - A click from Action Center after the popup has gone.
   - Taskbar grouping with the pinned shortcut. If it splits, file a follow-up for the NSIS
     AUMID.
   - The dev build and the installed build don't share notification settings.

## Review gates

- Review once per PR, against `prd.md` R1–R6. The implementer hands over a checklist that maps
  each requirement to `file:line` and its test.
- PR-A touches persisted data (a migration): it gets its own review of the migration and the
  restart sweep.

## Rollback points

- PR-A: revert. The `session_activity` table stays unused.
- PR-B: revert. Windows goes back to having no notifications.
