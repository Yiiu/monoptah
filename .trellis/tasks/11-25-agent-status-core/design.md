# Design: unified agent status + Windows attention

Research this builds on: `research/state-plumbing.md` (every consumer, with file:line),
`research/windows-attention.md`, `research/orca-status.md`.

## 1. Boundaries

| Layer | Owns |
|---|---|
| `monocode-core::agent_status` (new) | The pure status model and `derive()`. No GPUI. Unit-tested on Linux. |
| `monocode-core::provider` (Claude adapter) | Correct turn end and background-task bookkeeping. Keeps the raw task kind. |
| `monocode-core::store` | The new `session_activity` table (turn end, outcome, seen_at). |
| `monocode-gpui::shell` | Builds a `StatusInput` per session from `Conversation`/`Metadata`, caches `AgentStatus`, tracks user presence, decides "seen". It is the only place that calls `derive()`. |
| GPUI consumers | Read `Shell::agent_status(id)` only. |
| `monocode-gpui::notifications::platform_windows` (new) | Toast, taskbar overlay, flash, sound. |

Behaviour gates that are not status display (Escape/stop, archive-stops-busy, cloud moves,
review locks, history trim) keep reading `busy`. They decide whether a process can be stopped,
not what the user sees.

## 2. The model (`crates/monocode-core/src/agent_status.rs`)

```rust
pub enum Activity {
    NeedsYou { kind: Ask },                 // Ask::{Approval, Question, Remote}
    Running { step: Option<String>, since: u64, stale: bool },
    BackgroundWait { reason: Wait, since: u64 },   // Wait::{Subtasks(u32), UsageLimit { resume_at: u64 }}
    Done { outcome: Outcome, at: u64, unread: bool }, // Outcome::{Completed, Failed, Interrupted}
    Idle,                                   // never ran (drafts, empty sessions)
}

pub struct AgentStatus { pub activity: Activity, pub shells: Vec<ShellTask> }

pub struct StatusInput {
    pub approvals: bool, pub questions: bool, pub remote_waiting: bool,
    pub streaming: bool,               // turn open and not yielded
    pub wake_tasks: u32,               // non-ambient subagent/workflow/monitor
    pub usage_limit_resume_at: Option<u64>,
    pub shells: Vec<ShellTask>,
    pub step: Option<String>, pub state_since: u64, pub last_event_at: u64,
    pub last_turn: Option<TurnEnd>,    // { at, outcome } from session_activity
    pub seen_at: Option<u64>,
    pub now: u64,
}

pub fn derive(input: &StatusInput) -> AgentStatus
```

Rules, in order:
1. If `approvals`, `questions` or `remote_waiting` is set → `NeedsYou`.
2. If `streaming` → `Running`, with `stale = now - last_event_at > 30 min`.
3. If `wake_tasks > 0` → `BackgroundWait(Subtasks)`; else if `usage_limit_resume_at` is set → `BackgroundWait(UsageLimit)`.
4. If `last_turn` is set → `Done`, with `unread = seen_at < last_turn.at`.
5. Otherwise → `Idle`.

`since` is the time the activity started. The shell keeps it per session and resets it only
when the activity variant changes (Orca's rule: the clock does not restart on every update).

A one-line `label()` produces the English row text used by both existing layouts:
`"Running tests · 4m"`, `"Working · 4m"`, `"No update in 30m"`, `"Waiting on 2 subtasks"`,
`"Usage limit · resumes in 23m"`, `"Done"`, `"Failed"`, `"Stopped"`. The relative time once the
row is read is still produced by the existing code. Step labels come from
`TranscriptView::live_turn` (`transcript.rs:1080`), which is provider-neutral.

## 3. Claude adapter (`provider.rs`, `provider/tasks.rs`, `provider/lifecycle.rs`)

- `BackgroundTask` gains `kind: TaskKind { Shell, Agent, Monitor, Workflow, Other }`, mapped
  from raw `task_type` (`local_bash` → Shell, `local_agent`/`remote_agent` → Agent; others are
  matched by substring, with `Other` treated as a wake task, conservatively). `agent: bool` is kept
  as `kind == Agent` for the existing shelf UI.
- **Turn end**: the `result` hold-back (`provider.rs:1085-1099`, `lifecycle.rs:77-91`) waits
  only for **wake** tasks (non-Shell kinds) and `claude_resume_at`. Shell-only background → emit
  `Event::Done` at `result`. Shell tasks keep being published through `Event::Background` after
  `Done`, so the chip stays live.
- **Authoritative list**: `background_tasks_changed` replaces `claude_tasks` **and** prunes
  `claude_background` and `claude_pending` to the ids in the list. `task_updated` with a terminal
  status removes the id from all three sets.
- **Wake after Done** (open question T1): a shell task that finishes after `Done` makes Claude
  take another turn. Before changing the hold-back, verify how the runtime handles assistant
  frames that arrive after `Done` (`provider/follow_up.rs` tests around `:435-478`). If they
  are not already treated as a new auto-started turn, add that: begin a turn without a user
  prompt, so status becomes `Running`. If this turns out unsafe, fall back to keeping the
  hold-back for shells but deriving `Done` in the status function when only shells remain (the
  provider state stays "open"; the user-facing status is still correct).
- Hermes (`acp.rs` `Stage::Background`) maps to `wake_tasks = acp.background.len()`. Codex
  has no background tasks.

GPUI side: an `Update.background` change now emits `SessionChanged`
(`conversation.rs:1896-1900`, emit conditions at `:1983-1988`), so status refreshes immediately.
`Conversation` exposes `yielded()` (result seen, waiting on tasks) so `streaming = busy &&
!stopping && !yielded`.

## 4. Persistence (`session_activity` table)

A new native-only table, so the `sessions` table shared with the desktop app is not touched:

```sql
CREATE TABLE IF NOT EXISTS session_activity (
  session_id TEXT PRIMARY KEY,
  turn_ended_at INTEGER, outcome TEXT, seen_at INTEGER
);
```

- Written when a turn ends (outcome from `Done` / failure / stop) and when the session is marked
  seen. Writes are cheap single-row upserts, done off the main thread with the existing store
  executor pattern (see `.trellis/spec/native/state-and-async.md`).
- **Migration**: when the table is created, insert a row for every existing session with
  `turn_ended_at = seen_at = now`. All existing sessions start read.
- **Interrupted on restart**: at startup, a session listed in `in_flight_sessions` (or with a
  turn begun after its last `turn_ended_at`) whose process is gone gets `outcome = Interrupted`,
  `turn_ended_at = now`, unread.
- `Shell.unseen_finished` is removed. Its role is taken by `derive()` reading `seen_at`.

## 5. Seen rule (shell)

`seen` holds when all three are true:
- the session is the visible pane: `workspace.active == Some(id)` and no app view, or it is
  shown in a split;
- the window is active (`window_active`, `shell.rs:9634`);
- `now - last_input_at < 180 s`.

`last_input_at` is updated by root-level mouse-move, mouse-down and key-down listeners on the
shell. When a Done session becomes seen, `seen_at = now` is written. A notification click that
opens a session also marks it seen. The rule is re-evaluated on `SessionChanged`, on activation
changes and on input after an idle period. This replaces the clear-on-focus logic in
`refresh_session_mark` (`shell.rs:2027-2042`).

## 6. Consumer migration

`Shell::agent_status(id) -> &AgentStatus` is cached and recomputed in `refresh_session_mark`,
which becomes `refresh_status`. `busy_sessions` and the session marks are derived from it.
Every display consumer in `research/state-plumbing.md` §1 switches to it:

| Consumer | Maps from |
|---|---|
| Sidebar row text, both layouts (`rails.rs:728-783`, `shell.rs:7402-7470`) | `label()` plus the existing "Draft" and relative time |
| Session/tab marks (`session_mark::State`) | NeedsYou → Attention, Running → Working, BackgroundWait → Working without spin (new `Waiting` state), Done+unread → Done, else Idle |
| Filters (`session_filters`) | working = Running or BackgroundWait; needs approval = NeedsYou; done = Done+unread |
| Folder rows, title tabs, hover previews | same mapping |
| Live agents card | Running, BackgroundWait, Done+unread |
| Turn-end notices (`shell/notifications.rs:119`, `:469`) | the transition into Done replaces `notification_state.responding` |
| Dock badge / taskbar badge | count of NeedsYou (unchanged meaning) |
| Inbox running counts (`busy_sessions`) | Running or BackgroundWait |
| Today rows (`today_sessions.rs`) | the mark mapping |
| Mono panels and cards (`shell/monos.rs:80-110`, `monos/panels.rs`, `monos/cards.rs`, `conversation/mono.rs:388`) | NeedsYou → needs_input, Running/BackgroundWait → busy |
| Mobile catalog and watch (`mobile_host.rs:744-762`, `:1795`) | `busy` = Running or BackgroundWait, `waiting` from NeedsYou; wire format unchanged |

The shell chip (`shells`) is rendered as a small count chip on the row in both layouts. The
tooltip lists the commands.

Refresh: rows showing a Running clock reuse the live card's tick scheduling
(`live_agents.rs:103-135`), throttled to about 15 s for the step text and to the minute for the
clock.

## 7. Windows attention

Follows `research/windows-attention.md` "Recommended minimal approach":

- **Identity**: `cx.set_app_identity(id, "Monoptah Native")` at startup on Windows. Use
  `com.monoptah.native` when `native-install.json` sits next to the exe, otherwise
  `com.monoptah.native.dev`.
- **`notifications/platform_windows.rs`** implements the existing free-function interface:
  - `show`: posts the toast through the host, because it needs `cx`. The title is
    `"{project} · {session}"`, the body is the clipped agent ask, the action is `Open`.
  - Clicks: the `on_system_notification_response` callback parses the tag with
    `Target::from_tag` (enabled for Windows) and queues it into the existing `drain_clicks`
    path, or calls `open_notice_target` directly.
  - `permission`: returns `Granted`.
  - `open_settings`: opens `ms-settings:notifications`.
- **Badge**: `set_dock_badge(count)` → `ITaskbarList3::SetOverlayIcon` on the HWND captured
  from the shell window (via `raw-window-handle`, stored once). The icon is a 16×16 HICON built
  at runtime from an RGBA buffer: a filled amber circle with a 3×5 bitmap digit, `9+` above 9.
  `NULL` clears it. The overlay is re-applied when the `TaskbarButtonCreated` message arrives,
  if that is reachable; otherwise on the next count change.
- **Flash**: `FlashWindowEx(FLASHW_TRAY | FLASHW_TIMERNOFG)` when a new NeedsYou notice is
  dispatched while the window is inactive.
- **Sound**: extend `sound.rs` to Windows: `PlaySoundW(path, SND_FILENAME | SND_ASYNC |
  SND_NODEFAULT)` on the cached cue WAV.
- **Dedupe**: one toast per session per 5 s (`delivery.rs`, a per-target timestamp map).
- **Dependencies**: `[target.'cfg(windows)'.dependencies] windows = "0.62"` with the features
  listed in the research doc, plus `raw-window-handle` for Windows. Update `Cargo.lock` (CI
  builds with `--locked`).
- The `cfg_attr(not(macos), allow(dead_code))` attributes become `not(any(macos, windows))`.
- **Installer AUMID**: not in this task. If manual testing shows the pinned shortcut and the
  running window as separate taskbar buttons, file a follow-up task for setting the AUMID on
  the NSIS shortcut.

## 8. Compatibility and rollback

- No wire-format change for mobile. No change to the shared `sessions` table.
- Rollback is a revert of the PRs. The `session_activity` table is left unused, which is
  harmless.
- The two PRs are independent: PR-B (Windows) uses the existing NeedsYou count and does not
  need PR-A to land first. Its dedupe and "seen on click" hooks attach to PR-A when present.

## 9. Risks

- **T1, wake after Done**: see §3. Verify first; the fallback keeps behaviour correct.
- **Monitor and workflow `task_type` strings** are not in the repo. Unknown kinds are treated
  as wake tasks, so they never cause a false Done.
- **Test churn**: about 25 tests pin current behaviour (`research/state-plumbing.md` §7). Each
  is updated to the new semantics, never weakened.
- **Windows code is only `cargo check`ed in CI**. Keep the logic portable (tag parsing, badge
  pixels, dedupe) and unit-test it on Linux. Win32 calls are thin wrappers. Testing on
  Windows 11 is manual.
