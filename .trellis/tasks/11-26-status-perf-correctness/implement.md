# Implement: status performance and correctness

Lane branch: `feat/agent-overview-tree-status-perf`, cut from `feat/agent-overview-tree` and merged back when done (see the parent `handoff.md`). Make all edits for a step first, then run fmt, clippy and the filtered tests once.

Steps, one commit each:

1. **R6 migration flag (core).** Add the backfill key and keep unread rows across a recreate. Also cover the GPUI retry path.
   - Tests: `store::session_activity` (recreate keeps unread; backfill only once).
2. **R7 limits (core).** Card option cap with an error, `decision_cards` pruning, and a snapshot without archived sessions.
   - Tests: `mono::cards`, `mono::patrol`, `mono::heartbeat`.
3. **R1 step throttle (GPUI).** Cache the step label by tool id in `agent_activity.rs`, and add the early return in `live_turn`.
   - Tests: a parse counter, plus the existing `agent_activity` and `live_agents` tests.
4. **R2 timers (GPUI).** Mono clock early exit, usage-limit wake schedule, and the worktree check paused while the window is inactive.
   - Tests: `monos::scheduler`, `agent_activity`, `project_tree`.
5. **R3 terminal stale (GPUI and core `StatusInput`).** Add `stale_applicable` and terminal output timestamps.
   - Tests: `agent_status` (core), `tui_session`.
6. **R4 legacy cards (GPUI).** Implement in `mono_decisions.rs`.
   - Tests: a fixture with an old card followed by a user message, which must not be Needs you.
7. **R5 Mono sessions tool (GPUI).** Implement in `app_commands.rs`.
   - Test: a dev-server-only session returns `busy: false`. Note that some `app_commands` tests already fail on Windows on `main`; run this one specifically.
8. **R8 summary queue (GPUI).** Implement in `progress_summary.rs`.
   - Test: 6 turn ends, all 6 receive model text.

Commands, from `crates/monocode-gpui`:

```bash
cargo fmt --all
cargo +1.98.1 nclippy -p monocode-core -p monocode-gpui --all-targets -- -D warnings
cargo +1.98.1 ntest -p monocode-core <filters> -- --test-threads=1
cargo +1.98.1 ntest -p monocode-gpui --bin monocode-gpui <filters> -- --test-threads=1
```

Judge clippy by your own files only; the Windows-only lints are known and pre-existing.

Hand-over: a checklist that maps R1–R8 to `file:line` and its test. Merge the lane into `feat/agent-overview-tree` and push.

Rollback: each step is its own commit, so revert the one at fault.
