# Windows provider wire drop must not block the turn

## Problem

On Windows, a Codex session stayed "running" forever after Codex had finished the turn
(rollout `task_complete` at 19:03:13, session `4c3a92cf…`, 2026-11-27). Its queued follow-up
was never sent.

Evidence gathered on the affected machine:

- `idle_agents = 0`, so `parked::insert_parked` returns the wire and `parked::put` drops it
  on the turn's own thread.
- The Windows `impl Drop for Wire` (`crates/monocode-core/src/provider/parked.rs`) terminates
  the child, then joins the stdout and stderr reader threads with no bound.
- During the turn Codex ran `npm run native:dev -- --detach`. Windows passes every inheritable
  handle down `CreateProcess` chains (codex → powershell → npm → node → app), so the detached
  Native Dev app held the write end of the provider's stdout pipe. It had four inheritable
  unnamed pipe handles although its own stdio goes to a log file.
- The stdout reader never saw EOF, the join never returned, `run_operation` never returned,
  and `session_activity.turn_ended_at` stayed empty. Stopping the Native Dev process ended the
  turn immediately (`turn_ended_at` 19:46:28, `outcome = completed`).

The Unix `Drop` already does its joins on a background thread, and the doc comment on
`impl Drop for Wire` promises that "turn completion never wait[s] on this". That promise is
false on Windows.

## Requirements

1. Dropping a `Wire` on Windows returns promptly even when a grandchild process still holds
   the provider's stdout or stderr pipe. The child is still terminated before `drop` returns.
2. The reader threads of a retired wire are not leaked for the lifetime of such a grandchild:
   after a short grace their pipe I/O is cancelled (`CancelIoEx` on duplicated pipe handles)
   and they are joined off the caller's thread.
3. Normal retirement (no grandchild) keeps working: buffered output is not cut off before the
   grace period and threads end.
4. Unix behaviour is unchanged.

## Acceptance criteria

- A Windows-only test spawns a provider child that starts a long-lived grandchild inheriting
  its stdio pipes, then exits. Dropping the wire returns in well under the grandchild's
  lifetime, and the reader thread finishes shortly after.
- The test fails on the current code (drop blocks until the grandchild exits).
- `cargo fmt --check`, clippy `-D warnings` for `monocode-core`, and the `provider::parked`
  tests pass on Windows.

## Out of scope

- Detecting a provider that exits mid-turn while a grandchild holds its pipe (the turn loop
  waits for `Disconnected`). Using child exit as a signal could end healthy turns behind
  launchers that exit early, so it needs its own design.
- `provider/discovery.rs` and `provider/opencode.rs` reader joins (one-shot probes).
