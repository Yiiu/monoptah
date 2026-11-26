# Design: status performance and correctness

The facts behind each point are in `11-25-agent-overview/research/audit-status-mono.md`, and the original status design is in `11-25-agent-status-core/design.md`.

## R1 Throttled step text

- **Current behaviour.** `gpui/conversation.rs:2089` and `gpui/shell/notifications.rs:721` call the status refresh on every provider batch. That refresh calls `TranscriptView::live_turn` (`gpui/transcript.rs:1085-1120`), which reverse-scans the transcript and deserializes the last `"tool"` row each time.
- **Change.** Split the refresh into two parts:
  1. **Activity.** Recompute it on every relevant event, as today. `agent_status::derive` is cheap because it only compares inputs.
  2. **Step label.** Cache it per session, keyed by the last tool row's id, and parse only when that id changes. A per-session `next_step_refresh` instant limits label updates to at most one every 15 s, unless the tool id changed.
- **Where.** Keep the cache next to the activity cache in `shell/agent_activity.rs`. `live_turn` takes the cached tool id as input and returns early when it has not changed.
- **Test.** Feed N text batches and one tool call, then assert the parse counter is 1, using a `#[cfg(test)]` counter.

## R2 Timers

- **Mono clock** (`gpui/monos/scheduler.rs:1485-1545`). Compute the earliest due time across all Monos without building their views. Build views only for Monos whose patrol is due. If none is due, return before any per-session work.
- **Usage-limit wait** (`gpui/shell/agent_activity.rs:111-122`). Choose the next wake from the time remaining, `remaining = resume_at - now`:

  | Remaining | Next wake |
  |---|---|
  | more than 10 minutes | 60 s |
  | 1 to 10 minutes | 15 s |
  | under 1 minute | 1 s |

  Labels change by the minute, so nothing visible changes.
- **Worktree check** (the 75 s check in the Tree). Skip it while `window_active == false`, and run it once when the window activates.

## R3 Terminal stale

- **Cause.** TUI sessions only emit busy and idle transitions (`gpui/tui_session.rs:437-442`), so `last_event_at` falls back to the turn start.
- **Change.**
  - When the session's terminal grid receives output, update `last_event_at`. Throttle this to one timestamp per second; the terminal already redraws on output.
  - If the terminal view is not available, for example a background TUI session, mark the session's `StatusInput` as `stale_unknown`. `derive` then never returns `stale` for it.
- **Contract.** `StatusInput` gains a field `stale_applicable: bool`, which defaults to `true`. TUI sessions without output tracking set it to `false`.

## R4 Legacy choice cards

- A card counts as answered when either of these is true:
  1. a reply is attached to it (the current rule), or
  2. a user message appears after it in the Mono transcript and the card predates this release.
- Detect "predates" with the card's timestamp versus the `session_activity` migration time. Alternatively, add a `legacy: true` flag that a one-time pass sets on load for every existing unanswered card that has a later user message.
- Implement this in `gpui/transcript/mono_decisions.rs:44,113-124`. Test with a fixture transcript.

## R5 Mono sessions tool

- `shell/app_commands.rs:486` builds its session JSON from `Metadata.busy`. Change it to use `self.agent_status(id).is_busy()`; Running and BackgroundWait count as busy. The JSON shape does not change.

## R6 Migration re-run

- Track completion in `native_settings` under the key `session_activity_backfilled_at`. Fill "all read" only when that key is missing, and set it in the same transaction.
- If the table is recreated later, leave rows absent. A missing row means "no turn recorded", which shows as Idle or read, but it does not overwrite an unread row that still exists in memory. After recreation, write the in-memory unread set back.
- Implement this in `core/store/session_activity.rs:34-62` and the retry path in `gpui/shell/agent_activity.rs:54-61`.

## R7 Growth limits

- **Choice cards** (`core/mono/cards.rs:129-134`). Allow at most 12 options. Return `Err("choices card supports at most 12 options")` so that the Mono sees the error.
- **`decision_cards`** (`core/mono.rs:72`). Prune answered cards older than 7 days and cards whose session is archived. Keep at most 200.
- **Patrol snapshot.** Skip archived sessions when building `heartbeat.json`.

## R8 Summary queue

- **Cause.** `shell/progress_summary.rs:19,122-124` drops to fallback text above 4 in flight.
- **Change.** Replace the drop with a FIFO queue. When a summary finishes, start the next one. Queued entries still count as pending for the row, which keeps showing its fallback until the model text arrives. Dedupe queued entries by session id and keep only the latest turn.

## Compatibility

- There are no wire-format changes. Mobile, the app commands JSON and the store keep their shapes.
- Rollback: revert the commits. The new settings key is harmless if it is left behind.
