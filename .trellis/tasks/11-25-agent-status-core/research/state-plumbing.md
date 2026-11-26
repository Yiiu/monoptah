# Research: agent status state plumbing (native GPUI app)

- **Query**: Where every "working / needs input / done / unseen" consumer reads its state; how needs-input, Done(unseen), tool-step labels, turn start time, Claude task frames and session persistence work today; which tests pin current behaviour.
- **Scope**: internal (crates/monocode-gpui, crates/monocode-core; TS upstream adapter quoted for comparison only)
- **Date**: 2026-11-25

Paths are relative to `crates/monocode-gpui/src/` (GPUI) or `crates/monocode-core/src/` (core) unless written in full.

---

## 0. The primitive signals that exist today

| Signal | Where it lives | Meaning |
|---|---|---|
| `Conversation.busy: bool` | `conversation.rs:265` | Set true in `begin_turn` (`conversation.rs:2012`); false on the final `Update` (`conversation.rs:1922`) and on `stop()` (`conversation.rs:3270`). Stays true while Claude holds back `Event::Done` for background tasks, and while Hermes is in `Stage::Background`. |
| `Conversation.stopping: bool` (private) | `conversation.rs:273` | Set by `stop()` (`conversation.rs:3269`), cleared at `begin_turn` / final update. |
| `Conversation::responding()` | `conversation.rs:464-466` | `self.busy && !self.stopping` |
| `Conversation::needs_approval()` | `conversation.rs:461-463` | `!self.approvals.is_empty() \|\| !self.questions.is_empty()` |
| `Conversation.approvals: VecDeque<Approval>` | `conversation.rs:314` | Filled from `Update.approvals` (`conversation.rs:1901-1906`), cleared at finish (`:1955`) and stop (`:3282`). |
| `Conversation.questions: VecDeque<(Entity<QuestionCard>, Vec<Subscription>)>` | `conversation.rs:318-319` | `push_question` from `Update.questions` (`:1908-1910`); `cancel_questions` at finish/stop. |
| `Conversation.status: String` | `conversation.rs:259` | Free text: "Ready", "Waiting for approval", "Waiting for your answer", "Waiting for background tasks…" (from core `provider.rs:1098`), "Compacting context…" etc. Written at `conversation.rs:1893-1915`. |
| `Conversation.background.tasks: Vec<BackgroundTask>` | `conversation/background_tasks.rs:13-17`, field `conversation.rs:241` | Set from `Update.background` (`conversation.rs:1896-1900`, which also calls `transcript.set_yielded(count)`); cleared by `clear_background()` at finish (`:1924`) and stop (`:3281`). **Not** emitted as `SessionChanged` by itself. |
| `Conversation.notification_cancelled` | `conversation.rs:296` | Stop marks the turn as cancelled so no "finished" notice is sent. |
| `Conversation.settings.usage_limit` / `auto_resume_state()` | used by `mono_status` and sidebar `auto_resume` | Usage-limit wait. |
| `Shell.busy_sessions: HashSet<i64>` | `shell.rs:856` | Maintained only in `refresh_session_mark` (`shell.rs:2029-2037`); pushed to Inbox via `push_inbox_busy` (`shell.rs:2061-2066`) and `app_views.rs:443`. |
| `Shell.unseen_finished: HashSet<i64>` | `shell.rs:860` | In-memory only; see section 3. |
| `Shell.session_marks` / `Shell.tab_marks: HashMap<i64, Entity<SessionMark>>` | `shell.rs:847-848` | `session_mark::State {Idle, Working, Done, Attention}` (`session_mark.rs:49-56`), set in `refresh_session_mark` (`shell.rs:2043-2057`). Sidebar mark never gets `Attention`; tab mark does. |
| `Shell.remote_marks: HashMap<i64,(bool,bool)>` | `shell.rs:744` | `(running, needs_input)` for cloud sessions, from cloud snapshot `activity` (`shell/cloud.rs:101-103`, applied `shell/cloud.rs:1179-1182`). |
| TUI busy | `shell/tui_sessions.rs:72-76` `terminal_busy()` | Claude-hook spool events (`shell/tui_busy.rs`), never saved. No approvals, no tool info. |
| `session_loading::Metadata` | `shell/session_loading.rs:4-31, 33-46` | The closest thing to a central accessor today. `busy = live.busy \|\| terminal_busy \|\| remote_marks.0`; `needs_approval() = live.needs_approval() \|\| remote_waiting`; `responding() = live.responding() \|\| terminal_busy`; `auto_resume()`, `has_saved_draft()`. Note `busy` includes remote running, `responding()` does not. |
| `notification_state.responding: HashSet<i64>` | `shell/notifications.rs:41` | A *second*, independent "was responding" set used to detect turn end for notices. |

`refresh_session_mark` (`shell.rs:2001-2059`) is the one place that derives Working/Done/Idle/Attention. It is called from the `SessionChanged` subscription (`shell.rs:1930`), `persist()` for the active session (`shell.rs:4764`), `archive_family.rs:137`, notification-chat subscription (`shell/notifications.rs:753`), and `tui_sessions.rs:202`; `refresh_session_mark_idle` (`shell/tui_sessions.rs:181-195`) forces Idle when a TUI pane is killed.

```rust
// shell.rs:2028-2041
if busy {                       // busy = session.responding()
    self.busy_sessions.insert(id);
    self.unseen_finished.remove(&id);
} else if cancelling {          // cancelling = session.busy
    self.busy_sessions.remove(&id);
    self.unseen_finished.remove(&id);
} else if self.busy_sessions.remove(&id) && !focused && !archived {
    self.unseen_finished.insert(id);
}
if focused || archived {
    self.unseen_finished.remove(&id);
}
```
(`focused = !self.app_view_open() && self.workspace.active == Some(id)`, `shell.rs:2027`.)

---

## 1. Consumers of "working / needs input / done / unseen"

### Sidebar

| Consumer | file:line | Reads |
|---|---|---|
| Classic row status text ("Needs input"/"Need approval", "Working...", auto-resume, "Done", "Draft", relative time) | `shell/rails.rs:728-783` | `Metadata::needs_approval()`, `orchestration_summaries.contains_key` (label only), `Metadata::responding()`, `session_marks[id]` (spinner), `Metadata::auto_resume(now)`, `unseen_finished.contains`, `Metadata::has_saved_draft()`, `summary.updated_at` |
| Classic row uses it | `shell/rails.rs:430`, `:491` | via `classic_session_status` |
| Classic row approval flag | `shell/rails.rs:370` | `Metadata::needs_approval()` |
| New-layout row suffix ("Needs approval", "Working…", auto-resume, "Done", "Draft") | `shell.rs:7402-7470` (`session_suffix`, called `shell.rs:7151`) | same set as classic; "Done" colour `0x2dd4bf` vs classic `0x34d399` |
| New-layout row provider mark (spinner/check) | `shell.rs:6966-7040` (`session_row_with`) | `session_marks[id]` entity (state set by `refresh_session_mark`) |
| Orchestration agent rail rows | `shell/rails.rs:832-845` (needs input, `working = summary.live && task.status=="running" && !needs_input`), `:970-976` (`running`) | `Conversation::needs_approval()`, `Conversation::responding()`, orchestration `summary.live` / `task.status` |
| Started-rail "in play" / auto-expand | `shell/rails.rs:85-104`, `:193` | `Conversation::responding()` via `Shell::responding` |
| Sidebar filter (status: working / needs approval / done) | `shell/session_filters.rs:103-115`; matcher `monocode-core/src/session_filters.rs:50-64` | `Metadata.busy` (not `responding()`), `Metadata::needs_approval()`, `unseen_finished.contains` |
| Folder/group row (new layout) | `shell/session_groups.rs:1687-1690` | any `needs_approval()`; any `unseen_finished` |
| Folder/group row (classic) | `shell/session_groups.rs:1874-1885` | any `needs_approval()`, any `responding()`, first responding session's `session_marks` entity, any `unseen_finished` |
| Rename row | `shell/session_groups.rs:1499-1503` | `needs_approval()` |
| Classic sidebar tab sessions | `shell/classic_sidebar.rs:846` | `Metadata.busy` |
| Classic sidebar hover preview | `shell/classic_sidebar.rs:961-964` | `unseen_finished.contains` -> `Conversation::preview(done)` |
| Conversation preview header | `conversation.rs:543-560` (`preview(done)`) | `needs_approval()`, `responding()` (colour), `busy` (text "Working…"), `done` arg, `loading` |
| Hover preview (new layout) | `shell.rs:7856-7861` | `unseen_finished.contains` |
| Title tabs lit/unlit | `shell.rs:8163-8166`, rendered `shell.rs:8220`, `:8431` | `Metadata::responding() \|\| needs_approval() \|\| unseen_finished` + `tab_marks[id]` |
| `sidebar_list.rs` | `shell/sidebar_list.rs:518`, `:532` | No direct status reads; delegates to `session_card` / `nested_session_card` (`shell/rails.rs:259-300`), which dispatch to classic (`rails.rs`) or new (`shell.rs session_row_with`). |
| `project_rail.rs` | — | **No status consumer found** (only project marks/logos). |

### Live agents card

`shell/live_agents.rs:55-90`:
```rust
let done = !view.busy && self.unseen_finished.contains(id);
if view.summary.archived || !(view.responding() || done) { return None; }
let (tool, started_at, duration_ms) = view.transcript.read(cx).live_turn(&view.summary.cwd);
let needs_approval = view.needs_approval();
// activity: "Need approval" | "Done" | tool label | "Working"
agents.sort_by_key(|a| (!a.needs_approval, a.done, a.started_at.unwrap_or(u64::MAX)));
```
Only loaded sessions (`self.sessions`), not TUI or remote. Ticks with `live_next_change` / `live_tick` (`live_agents.rs:103-135`). Gate: `workspace.preferences.working_agents && workspace.sidebar_activity_visible`, min 2 agents (`LIVE_AGENT_MIN`).

### Notifications, dock badge, approvals

| Consumer | file:line | Reads |
|---|---|---|
| Turn-end "agentFinished" notice (event path) | `shell/notifications.rs:119-153` (`notification_session_changed`) | `responding()`, `notification_state.responding` set, `busy`, `notification_was_cancelled()`, `summary.archived`, Mono-session check |
| Turn-end notice (per-frame path) | `shell/notifications.rs:469-530` (`sync_notices`) | same; `completed` vs `quiet` (`mobile_turn_ended`) |
| Input announcements ("agentInput") + pending cards | `shell/notifications.rs:395-444`, `:546-603` | `Conversation::pending_notification_inputs(cx)` (`conversation/notifications.rs:10-27`: approvals then questions), wrapped in `SessionInputs` (`notifications/approvals.rs:23-36`) |
| Dock badge | `shell/notifications.rs:416-424`, `:529-537`; count `notifications/delivery.rs:142-149` (`dock_badge_count`: sessions with non-empty pending) ; OS call `notifications/platform.rs:47` (no-op on non-macOS), `notifications/platform_macos.rs:205-213` | `pending.is_empty()` per session. Counts needs-input only, not Done. |
| Mute-rule redraw | `shell/notifications.rs:358-366` | any `needs_approval()` |
| Notice suppression | `notifications/delivery.rs:65-74` `should_notify`; `shell/notifications.rs:446-467` `notice_session_visible` | window focus, visible pane |
| Non-macOS delivery | `notifications/platform.rs:24-53` | `initialize()` false, `permission()` Unsupported, `show()` Err, `set_dock_badge` no-op. **Windows has no system notifications, badge or flash today.** |
| In-chat approval cards | `conversation/approvals.rs` (UI only; `view.busy = true` at `:160` is a test) | `approvals` deque |

### Mobile / web

| Consumer | file:line | Reads |
|---|---|---|
| Catalog session card `busy` / `waiting` | `shell/mobile_host.rs:744-762` -> `mobile/catalog.rs:55-56`, JSON at `mobile/catalog.rs:134-143` | `Conversation.busy` (raw, not `responding()`), `Conversation::mobile_waiting(cx)` (`conversation/mobile.rs:476-485`, Approval before Question) |
| Watch snapshot header (`busy`, `waiting` text) | `shell/mobile_host.rs:1795-1815`, `:1658`; `mobile/transcript.rs:652`; core `mobile/watch.rs:9,20,143,177` | `chat.busy`, `mobile_waiting` -> "Waiting for approval"/"Waiting for answers" |
| Mono live card | `shell/mobile_host.rs:878-889` | `chat.mono_status()`, `chat.busy` |
| Workspace poll keeps busy sessions | `monocode-core/src/mobile/catalog.rs:10-30` | `session["busy"] == true` |
| Push notification status | `monocode-core/src/mobile/notify.rs:196-201` | category `agentFinished`->`finished`, `agentInput`->`needs_input` |
| App-commands session list JSON | `shell/app_commands.rs:486`, `:639` | `Metadata.busy` |
| Mono host session list JSON | `monos/host.rs:1026` | `views[].busy` |
| Cloud lifecycle | `mobile/cloud/instances.rs:214`, `mobile/cloud/lifecycle.rs:215-224` | `session.busy` |

### Monos

| Consumer | file:line | Reads |
|---|---|---|
| `MonoSession` snapshot | `shell/monos.rs:3-18`, built `shell/monos.rs:80-110` | `busy = Metadata.busy \|\| mono_external_pending() \|\| mono_stopping() \|\| cloud_launch_pending()`; `needs_input = Metadata::needs_approval()`; `start_failed` |
| Sub-agent panel cards | `monos/panels.rs:311-318` (`AgentState {Working, NeedsInput, StartFailed, Idle(u64), Unavailable}`), `:347-369`, mapping `:1396-1403`, sort `:1358-1366` | `MonoSession.needs_input`, `.start_failed`, `.busy`, `.updated_at` |
| Launched session cards | `monos/cards.rs:372-386` (`launched_state`: input/working/draft/stopped/failed/done), `:415-427`, label map `:151`, `:264-269` | `needs_approval()`, `busy`, `has_messages`, `external_outcome()` |
| Mono chat status | `conversation/mono.rs:388-396` (`mono_status`: needs-you / working / idle) | `needs_approval() \|\| settings.usage_limit`, `responding()` |
| Scheduler attention | `monos/scheduler.rs:293-318` | `needs_approval()` |
| Workflow runner | `monos/workflow_runner/host.rs:295`, `:963` | `needs_approval()` |

### Today / Inbox

| Consumer | file:line | Reads |
|---|---|---|
| Inbox running counts / spinners | `inbox.rs:73`, `:391-396` (`set_busy`); `inbox/running.rs:20-28`; `inbox/ui.rs:606`, `:777`, `:994` | `Shell.busy_sessions` (pushed `shell.rs:2061-2066`, `shell/app_views.rs:443-447`) |
| Today session rows | `shell/today_sessions.rs:16-42` | `tab_marks[id].state()` (so includes Attention/Done), fallback `Metadata.busy` -> `MarkState::Working` |
| Today habit busy | `today.rs:75`, `:748-764` | Mono habit `busy` (separate) |

### Escape / stop

| Consumer | file:line | Reads |
|---|---|---|
| Escape stop | `shell/escape.rs:61-101` -> `Conversation::stop_if_busy` (`conversation.rs:3256-3263`) -> `stop()` (`:3264-3290`) | `busy` (a session whose Done is held for background tasks is stoppable by Escape) |
| Composer Stop vs Send | `conversation.rs:3841-3843` | `responding() && !has_value` |
| Archive stops a busy session | `shell/archive_family.rs:125-137` | `stop_if_busy` then `refresh_session_mark` |
| Archive shortcut blocked | `conversation.rs:451-459` | `needs_approval()` |
| Other `busy` gates (not status UI) | `shell/project_removal.rs:167-171`, `shell/cloud.rs:1627`, `:1761`, `shell/cloud_continue.rs:255`, `shell/cloud_move_api.rs:184`, `shell/cloud_app_archive.rs:497`, `shell/cloud_auto_stop.rs:141-145`, `shell/orchestration.rs:64`, `:140`, `shell/remote_sessions.rs:112`, `shell/accounts.rs:119`, `shell.rs:1984-1999` (`sync_review_locks`), `shell/session_loading.rs:75` (history trim) | `busy` / `Metadata.busy` / `busy_sessions` |
| Commands/history/edit gates | `conversation/commands.rs:84`, `conversation/history.rs:129`, `conversation/edit_last_turn.rs:73` | `needs_approval()` |

---

## 2. How "needs input / approval" is determined

**GPUI**: `Conversation::needs_approval()` = non-empty `approvals` or `questions` (`conversation.rs:461-463`). Remote/cloud sessions add `remote_marks.1` via `Metadata::needs_approval()` (`shell/session_loading.rs:18-20`). TUI sessions never need approval (`shell.rs:2015-2023`).

**Transport**: core emits `Event::Approval(Approval)`, `Event::ApprovalResolved(id)`, `Event::Question(Question)`, `Event::QuestionResolved(id)` (`provider.rs:111-114`). The runtime collects them into `Update.approvals/approvals_resolved/questions/questions_resolved` and sets `status` (`runtime.rs:1065-1082`). The conversation applies them at `conversation.rs:1901-1915` (ignored while `stopping`).

**Core storage** (per running `Protocol`): `approvals: HashMap<String,(Value,String,Value)>` (`provider.rs:307`), `questions: HashMap<String,(Value,String,Value,Question)>` (`provider.rs:314`), `question_order`, `visible_question` (one question shown at a time, `provider/questions.rs:164-184`). Questions may auto-resolve after 120 s when non-blocking (`questions.rs:177-179`, `expire_questions` `:186`).

Per provider (all funnel through `Protocol::approval` `provider.rs:1170-1250` or `Protocol::question` `provider/questions.rs:5-26` unless noted):

| Provider | Approval frame | Question frame |
|---|---|---|
| Claude | `control_request` / `can_use_tool` (`provider.rs:1131-1156`) | same frame with `tool_name == "AskUserQuestion"` (`provider.rs:1134-1143`) |
| Codex | method ending `/requestApproval`, `execCommandApproval`, `applyPatchApproval` (`provider.rs:831-850`); MCP `mcpServer/elicitation/request` via `codex_elicitation` (`provider/lifecycle.rs:6-39`) | `item/tool/requestUserInput` (`provider.rs:828-830`); async questions `provider/questions.rs:212-260` |
| ACP (Cursor, Grok, Antigravity, Fx, Hermes, Devin; `Kind::is_acp` `provider/kind.rs:86-88`) | `session/request_permission` (`provider/acp.rs:653-745`, emits `Event::Approval` at `:736`) | `cursor/ask_question`, `_x.ai/ask_user_question`, `x.ai/ask_user_question` (`provider/acp.rs:637-650`) |
| Pi / Omp | `confirm`-style method (`provider/pi.rs:610-631`) | `select`/`input`/`editor` (`provider/pi.rs:632-633`) |
| OpenCode | permission event -> `self.approvals` keyed `opencode-permission-{id}` (`provider/opencode/events.rs:463-480`) | `question.asked` / `form.created` (`provider/opencode/events.rs:483-506`) |

Auto-accept by access mode happens before emitting (e.g. `provider.rs:1218-1240`, `acp.rs:694-730`), so an auto-allowed request never reaches the UI.

---

## 3. "Done (unseen)" tracking

- Field: `Shell.unseen_finished: HashSet<i64>` (`shell.rs:860`, init `shell.rs:1380`).
- **Set**: only in `refresh_session_mark` when a session leaves `busy_sessions` while not focused and not archived (`shell.rs:2037-2038`). Tests insert directly (`shell/session_filters.rs:178`).
- **Cleared**: when it becomes responding (`shell.rs:2031`), on stop/cancel (`:2036`), when focused or archived (`:2040-2042`), session removal (`shell/session_groups.rs:1298`), TUI pane killed (`shell/tui_sessions.rs:185`).
- "Focused" = `workspace.active == Some(id)` and no app view (`shell.rs:2027`). Opening the session clears it via `persist()` -> `refresh_session_mark(active)` (`shell.rs:4763-4765`).
- `refresh_session_mark` is driven by `SessionChanged`. A `Background` update alone does not emit `SessionChanged` (`conversation.rs:1896-1900`; emit conditions at `:1983-1988`), so status changes caused only by background tasks do not reach the marks until another event.
- **Persistence: none.** No DB column, table or settings key; grep for `unseen`/`seen_at`/`last_seen` in `monocode-core/src/store*` finds nothing related. Every restart starts with an empty set. (`monocode-core/src/inbox.rs:417` `unseen()` is an Inbox-item concept, unrelated.)
- Separately, `notification_state.responding` (`shell/notifications.rs:41`) tracks turn end for notices, also in memory.

---

## 4. Provider `Event` enum, tool frames, turn start time

`Event` (`provider.rs:94-134`):
`Ready, Commands, FollowUp, FollowUpFailed{id,error}, SessionId, TurnId, Model, Context{used,window}, Metrics, Approval, ApprovalResolved, Question, QuestionResolved, BeginAssistant, Text, Reasoning, Tool(ToolCall), Status(String), Background(Vec<BackgroundTask>), UsageLimited, Interjection{..}, Goal, ConfigChanged{..}, Done`.

`BackgroundTask` (`provider.rs:136-146`): `id, description, agent: bool, command: Option<String>, started_at: u64`.

`ToolCall` (`tool.rs:21-34`): `id, name, input (JSON string), output, state: ToolState{Running,Completed,Failed,Cancelled}, panel: Option<tasks::Panel>, display: Option<Value>`. `ToolCall::presentation_in(cwd)` (`tool.rs:91-93`) -> `ToolPresentation {action, target, file_path, start_line, command, description, steps}` (`tool/presentation.rs:6-19`, `present()` at `:71`). `ToolCall::summary()` (`tool.rs:75-85`) returns ("Tasks", progress) / ("Agent", title) / (action, target).

Emit sites: Claude/Codex `provider.rs:785, 810, 904, 921, 955, 1013, 1439`, `provider/tasks.rs` (subagent trails), ACP `provider/acp.rs:1033, 1094`, Pi `provider/pi.rs:593`, OpenCode `provider/opencode/events.rs:641, 718`, compaction `provider/compaction.rs:44`. All arrive at `runtime.rs:946-968`, which serializes the `ToolCall` JSON into a transcript row with role `"tool"` (one row per tool id, updated in place).

**Existing step label**: `TranscriptView::live_turn(cwd)` (`transcript.rs:1080-1116`) finds the last prompt row, takes the last `"tool"` row of that turn, deserializes it as `ToolCall`, and returns `(Some("<action> <target>"), prompt.sent_at, duration_ms if finished)`. It is provider-neutral (works for any provider that emits `Event::Tool`), costs one reverse scan plus one `serde_json` parse, and is used only by the live agents card (`shell/live_agents.rs:64-65`). It requires the history loaded in a `Conversation`; unloaded/metadata-only, TUI and remote-mark sessions have no tool info. Subagent child tools are nested in the parent Agent tool's `panel` (not top-level rows), so the label shows the parent ("Agent …").

**Turn start time**:
- `Row.sent_at` on the prompt row (native_rows column `sent_at`, migrated `monocode-core/src/store.rs:226-237`), used by `live_turn`.
- `TurnInfo.started_at / completed_at / duration_ms / duration_known` (`monocode-core/src/agent.rs:296-314`), set as `settings.turn_model.started_at = now_ms()` at send (`conversation.rs:1627-1635`) and kept in `settings.turns` (saved at finish `conversation.rs:1975`).
- `BackgroundTask.started_at` for each background task (`provider/tasks.rs:595`, `:641`).
- `Conversation.summary.updated_at` is bumped at begin and finish (`conversation.rs:2022`, `:1921`).

---

## 5. Raw Claude task frames: `task_type` and `ambient`

Both fields are present in what we receive and are read, but only partially kept.

Per-frame lifecycle (`provider/tasks.rs:465-570`):
```rust
if value["type"] == "system"
    && matches!(value["subtype"].as_str(),
        Some("task_started" | "task_progress" | "task_updated" | "task_notification"))
    && value["ambient"] != true          // ambient tasks are skipped entirely
{
    let patch = if subtype == "task_updated" { &value["patch"] } else { value };
    if patch["is_backgrounded"] == true { claude_background.insert(id, description) }
    else if patch["is_backgrounded"] == false { claude_background.remove(id) }
    if subtype == "task_started" { claude_pending.insert(id) }
    self.track_claude_task(id, subtype, value, patch);
    if subtype == "task_notification" {
        let background = claude_background.remove(id).is_some();
        claude_pending.remove(id);
        if background && self.claude_result_seen { claude_resume_at = now + 15s }
    }
    ...
}
```
`track_claude_task` (`provider/tasks.rs:575-615`) pushes `BackgroundTask { agent: is_agent_task(value["task_type"]), command: <Bash input.command if tool_use_id is a Bash tool>, .. }` on `task_started`; removes on `task_updated` status completed/failed/killed/stopped or `task_notification`.

`background_tasks_changed` (`provider/tasks.rs:617-645`): filters `row["ambient"] != true`, then **replaces only `claude_tasks`**; `claude_background` and `claude_pending` are untouched.

`is_agent_task` (`provider/tasks.rs:681-686`): `local_agent | remote_agent` -> true; everything else (incl. `local_bash`, monitor, workflow, unknown) -> false. Raw `task_type` is discarded.

Done hold-back: `result` frame (`provider.rs:1085-1099`) and `lifecycle_tick` (`provider/lifecycle.rs:77-91`) emit `Event::Done` only when `claude_background` and `claude_pending` are empty and `claude_resume_at` passed. Otherwise status "Waiting for background tasks…". `sync_background` (`provider/tasks.rs:646-657`) publishes `claude_tasks` as `Event::Background` only after `claude_result_seen && !done`.

Consequences visible in code (facts, not recommendations):
- A `local_bash` task with `is_backgrounded: true` (dev server) sits in `claude_background` until its `task_notification`; `task_updated {status: completed}` removes it from `claude_tasks` but not from `claude_pending`/`claude_background`.
- Every non-ambient `task_started` enters `claude_pending`, regardless of type.

Task-type strings seen in the repo: only `local_bash`, `local_agent`, `remote_agent` (tests `provider/tasks.rs:984-985`; TS `src/integrations/harness/providers/claude/claudeProtocol.ts:628-631`). No literal for monitor or workflow task types exists anywhere in the repo. The TS adapter parses `taskType` and `ambient` on `task_started` (`claudeProtocol.ts:633-661`), `ambient` on `task_notification` (`:728-756`) and `background_tasks_changed` rows (`:762-786`); its comment says the list includes "subagents, shells, monitors" (`claude.ts:151-155`).

Other providers:
- Codex: no `Event::Background` emitted (known fact; Codex subagents are tracked as tool panels in `provider/tasks.rs`).
- Hermes (ACP): when `acp.background` is non-empty at prompt end it enters `Stage::Background` and withholds `Done` (`provider/acp.rs:914-923`); when dispatches finish, `acp_background` (`provider/acp.rs:1075-1115`) re-prompts Hermes itself. No `Event::Background` is emitted, so the UI only sees `busy`.

### T1 assistant output after a Claude result

`Protocol::frame` already recognizes a top-level assistant turn following a result and
clears `claude_result_seen`. It can only see those frames while the wire is being
read. `provider.rs` exits the receive loop as soon as `Protocol.done` is true.
The wire is either dropped or returned to `provider/parked.rs`, where frames
stay buffered until the next prompt or retirement at 180 seconds. The next
explicit prompt takes the parked wire with a fresh protocol. `runtime::run_turn`
also settles the transcript, checkpoints, and control receiver at this physical
end. The replay test in `provider/follow_up.rs` covers an empty result on
explicit resume. A spontaneous wake-up requires keeping the receive loop alive.

Use the status-only fallback. Publish a logical completion at the shell-only
result while the original worker keeps reading the live CLI and monitoring the
shell inventory. A top-level automatic assistant start publishes Running again;
the eventual wire exit retains the ordinary runtime finish path. Yielded status
uses only non-ambient wake tasks, with a short grace after their completion to
receive Claude's automatic continuation. A full inventory replaces task tracking
so a lost completion frame cannot hold the visible activity open.

---

## 6. Session persistence

- Production DB: `monocode.db` in the native data dir (`main.rs:377-381`, `data_dir().join("monocode.db")`), or the desktop app's DB with `--use-monocode` (`monocode-core/src/store/monocode_import.rs:17-32`, `<APPDATA or Library/Application Support>/com.monoptah.desktop/monocode.db`). Opened with `Store::open_monocode` (`main.rs:401`).
- Schema: `monocode-core/src/store/shared_schema.sql:2-15`, table `sessions(id TEXT PK, cwd, harness, model, model_settings, runtime_mode, title, provider_session_id, blocks_json, created_at, updated_at, branch, context_used, context_window, archived INTEGER, worktree_cwd, has_user_message, pinned, linked_work_item_json, provider_account_id, worktree_removed, is_draft, automation_id, queued_messages_json, queue_status, sidebar_hidden INTEGER, inbox_ask, surface)`. Also `in_flight_sessions(session_id, cwd, sort_index)` (`shared_schema.sql:21`), `session_reminders` (`:26`), `workspace_snapshot` (`:19`).
- Legacy/fixture schema (`Store::open`, `monocode-core/src/store.rs:164-213`): `native_sessions(id, title, provider, cwd, provider_id, updated_at, archived, pinned, work_item)`, `native_rows(..., sent_at)`, `native_settings(key, value)` (agent settings under key `agent:<id>`), `native_reminders`.
- In-memory struct: `store::Session` (`monocode-core/src/store.rs:86-102`): `id, title, provider, cwd, provider_id, has_messages, updated_at, archived, pinned, work_item, surface`.
- **Archive** exists: `Store::set_archived` (`store.rs:1249-1261`) + `record_archive_time`; UI `toggle_archive` (`shell.rs:2067+`), `shell/archive_family.rs`. Archived sessions are stopped and `provider::retire_session` is called (`archive_family.rs:125-136`).
- **Hidden** exists: `sidebar_hidden` column, read into `SessionMetadata.sidebar_hidden` (`monocode-core/src/store/session_metadata.rs:11`, `:97-98`; `store/shared.rs:2370-2385`), used for Mono-launched/worker sessions kept out of the sidebar.
- **Delete**: `Store::delete` (`store.rs:1468-1471`, "conversation, its settings, reminder and folder for good"); UI `Shell::delete_session` (`shell/session_groups.rs:980`), with `Shell.deleting` set (`shell.rs:882`).
- No status/seen column exists.

---

## 7. Tests that pin current Working/Done behaviour

| Test | file:line | Pins |
|---|---|---|
| `working_marks_animate_independently_and_completed_replies_clear_when_seen` | `shell.rs:10505-10552` | `busy=false` + `SessionChanged` puts id in `unseen_finished`; `open()` clears it |
| `title_tabs_draw_their_own_session_marks` | `shell.rs:11647-11669` | `tab_marks[a].state() == MarkState::Working` |
| `switching_layout_with_working_session_marks_keeps_cached_paint_valid` | `shell.rs:11671+` | working marks across layouts |
| `filters_persist_and_update_without_closing_the_menu_or_active_chat` | `shell/session_filters.rs:124-185` | filter "done" = `unseen_finished`, "working" = `busy` |
| `busy_marks_the_session_and_the_cli_exiting_clears_it` | `shell/tui_sessions/tests.rs:461` | TUI busy -> `Metadata::responding()`, MarkState::Working, `busy_sessions` |
| `killing_the_pane_clears_a_busy_mark` | `shell/tui_sessions/tests.rs:495-512` | `!unseen_finished.contains` after kill |
| `activity_visibility_switch_preserves_agents_and_survives_restart` | `shell/sidebar_list.rs:1268-1330` | live agents card with `busy=true` / `responding()` |
| `working_mark_spins_on_the_shared_clock_and_stops_after_completion` | `session_mark.rs:147` | `SessionMark` Working -> stop |
| `background_work_sits_on_the_ledge_steers_new_messages_and_clears_on_stop` | `conversation/background_tasks.rs:295` | `busy=true` + `set_background`; stop clears |
| `a_lone_task_folds_into_the_header`, `elapsed_reads_as_a_running_clock` | `conversation/background_tasks.rs:346`, `:369` | background shelf UI |
| `mono_reply_shows_while_the_run_waits_on_background_tasks` (+ neighbours using `set_yielded`) | `transcript.rs:7331-7370` | yielded run rendering |
| `claude_background_tasks_show_once_the_answer_yields_and_clear_as_they_finish` | `monocode-core/src/provider/tasks.rs:972-1014` | `Event::Background` contents, `agent` flag, `background_tasks_changed` clearing |
| `claude_task_ids_progress_and_reports_come_from_task_events` | `provider/tasks.rs:950` | task lifecycle frames |
| `claude_replayed_task_notification_result_does_not_end_the_turn`, `claude_turn_with_a_real_task_notification_run_still_ends`, `claude_turn_ends_when_a_queued_prompt_never_starts` | `monocode-core/src/provider/follow_up.rs:435`, `:468`, `:478` | Done hold-back / `claude_resume_at` |
| `Status` filter tests | `monocode-core/src/session_filters.rs:125-180` | `Status{working, needs_approval, done}` semantics |
| question API asserts | `conversation/question_api.rs:272`, `:314` | `needs_approval()` with questions |
| Escape tests | `shell/escape_tests.rs:71` (`finish_for_test("Done")`) | stop targets / busy |
| Inbox running | `inbox/running.rs:49`, `:56`; `inbox/ui.rs:3645` | `busy_sessions` -> running counts |
| Monos | `monos/cloud_tests.rs:1514`, `:1592` (`..._is_not_working`), `monos/cloud_tests.rs:266`, `monos/workflow_runner/tests.rs:132`, `:914` | `MonoSession.busy` / needs_input semantics |
| Mobile catalog | `mobile/catalog.rs:240-257` | `busy`, `waiting: "approval"` JSON |
| Mobile cloud | `mobile/cloud/lifecycle_tests.rs:227` | `busy` either side |

---

## Caveats / Not Found

- No Windows notification, taskbar badge or flash code exists; all non-macOS platform functions are stubs (`notifications/platform.rs:24-53`).
- No literal `task_type` values for monitor or workflow tasks exist in the repo; whether Claude sends `ambient` on monitors could not be confirmed from code (no web search tool available in this run).
- `responding()` vs `busy` is used inconsistently across consumers (filters, mobile, Monos, Today read `busy`; sidebar text, marks, live card, notices read `responding()`), noted as fact.
- Remote (`remote_marks`) and TUI sessions have no tool/step info and no start time in the shell.
- Did not inspect `runtime/remote.rs` (remote-host sessions) event flow in detail.

## PR-A local verification

- Branch `fix/agent-status-core`, in `.worktrees/agent-status-core`. R1?R5 are implemented. R6 stays deferred.
- On Windows with Rust 1.98.1, 117 core tests and 154 native script tests passed. The script suite skipped 65 platform-gated cases.
- The GPUI run passed 286 of 287 tests. The remaining Mono worktree-deletion fixture failed at the local Git push before it reached application logic. It passed on its own using a process-local Git URL rewrite for the `\\?\` prefix. The 75 focused GPUI tests and 7 additional layout, usage-limit and transcript tests passed.
- Formatting, Windows all-target GPUI check and Clippy passed. Clippy reported warnings in files that this task did not change.
- A real Claude GUI wake test and Linux CI are pending.
