# Research: R5/R7 Tree session row trailing area and shell chip

- **Query**: trailing status column, done caption, hover archive swap, read/unread/done source, tooltip for rows, shell chip and running terminals
- **Scope**: internal (origin/main 7106f9c1)
- **Date**: 2026-11-27

Paths under `crates/monocode-gpui/src/`.

## Findings

### `tree_session_row` (shell/project_tree/render.rs:895-1170)

| Lines | What |
|---|---|
| :910 | `hover = self.session_hover == Some(id)` |
| :913-916 | `meta = session_metadata(id)`, `status = agent_status(id)`, `needs_you = status.needs_you()` |
| :920-937 | `(look, badge) = tree_unread_look(id, &status.activity)` (0..1, fades); `unread = look > 0.`; `shown_activity` = Done with `unread` replaced by the look; `read_activity` = Done with `unread: false` |
| :938-947 | `subline`: needs_you -> ask label (`tree_ask`, :780-792) or "Waiting for input"; unread -> `progress_summary(id)`; else None |
| :948-982 | `trailing`: |
|  | `Done { at }` -> check/activity glyph + `tree_caption(monocode_core::time::relative(at, now), color)` (always, read or unread) (:949-968) |
|  | `NeedsYou` -> None (:969) |
|  | `Idle` -> reminder caption if any (:970-977) |
|  | `_` (Running/BackgroundWait) -> `tree_caption(status.label(now), activity_color)` e.g. "{step} · 3m" (:978-981; `agent_status.rs:162-207`) |
| :990-997 | avatar = `session_mark::provider_activity_icon(provider, &shown_activity, Sidebar, edge_fill, reduce_motion, badge)` (carries running/needs-you/unread badge) |
| :1013 | `compact = !needs_you && !unread` |
| :1014-1084 | `content_row` (24px): avatar, title (`min_w_0 truncate`, 12px, SEMIBOLD when look >= .5; Done ink 0.58..0.92), mono mascot, `flex_1` spacer, **branch caption** `branch.map(tree_caption(branch, content(0.45)))` (:1042), **shell chip** `self.shell_task_chip(id)` (:1043), **hover swap** `if hover { Some(session_archive_button(id, &meta, cx)) } else { trailing }` (:1044-1048), child-count chevron when `children > 0` (:1049-1084) |
| :1103-1123 | summary line under row when `!compact` (`tree-session-summary-{id}`), height `17 * summary` |
| :1125-1131 | `on_hover` -> `hover_session(id, bounds, entered, compact, ..)` unless a rail drag/pane drag is held |
| `render.rs:1202-1210` | `tree_caption`: `min_w_0 max_w(146) truncate 11px` |
| `render.rs:1212-1214` | `tree_dot(color)` 6px |

The title is `min_w_0` but the trailing pieces are siblings with fixed content; `tree_caption` max 146px.

`branch` on session rows: `TreeRow::Session.branch` = `show_branch.then(|| tree_branch(id))` (`project_tree.rs:1116`); `show_branch` is true for Pinned/Reminders sections (`(2, true)`, :1003) and Archived (:767), false for worktree/loose lists (:1041, :1066).

### Where read/unread/done are known

- `Activity::Done { outcome, at, unread }` from `monocode_core::agent_status` (`crates/monocode-core/src/agent_status.rs:37-57`); `at` = turn end ms; `unread = seen_at.is_none_or(|seen| seen < turn.at)` (:142-147).
- `Shell::agent_status(id)` (`shell/agent_activity.rs:113-115`) reads the `agent_statuses` cache built by `update_agent_status_for` (`agent_activity.rs:159-...`), restored from `session_activity` at startup (`agent_activity.rs:95-110`).
- Tree-only visual linger/fade: `tree_unread_look` (`project_tree.rs:949-959`), `tree_note_unread` (:851-901), `tree_settle_unread` (:917-945); `lingering`/`fades` in `TreeState` (:281-283).
- Counts: `Counts::add` (`project_tree.rs:28-35`) — Done{unread:true} counts as unread.

### Hover archive swap

- `session_archive_button(id, &meta, cx)` (`shell.rs:7372-7414`): 20px, `mr(-3)`, selector `archive-session-{id}`, aria "Archive {title}"/"Unarchive …", click `toggle_archive`. Also used by Focused `session_suffix` (`shell.rs:7419-7429`).
- `hover_session` (`shell.rs:6870-6920`) sets `session_hover`, notifies `sidebar_frame`, and for `compact` rows schedules the **session preview popover** (`schedule_preview`, `shell.rs:6817-6850`, ~120ms + scroll settle; skipped for terminal sessions or while a menu is open).

### Existing hover surface for rows (no per-row tooltip today)

- Tree session rows have no `.tooltip(..)`. Their hover surface is the session preview card rendered at `shell.rs:7967-8005` (`popover("session-preview", Anchor::Slot(preview.anchor))`, side Right, width 260):
  - unloaded session: `shell/session_preview.rs:11-29` — header label = `status.label(now)` else `relative(Done.at or summary.updated_at)`.
  - loaded session: `Conversation::preview(status)` (`conversation.rs:519-635`) — header label = `status.label(now)` else `relative(self.summary.updated_at)` (not the Done `at`), plus title, Branch, Worktree/Directory.
  - For read Done, `status.label()` is None (`agent_status.rs:205`), so the card shows a bare relative time like "1h 19m" (`time.rs:9-35`, formats "now", "{m}m", "{h}h {m}m", "{d}d", then a date).
- Tooltip views available: `cloud_mark::CloudTip` (`cloud_mark.rs:98-113`), `project_rail::ProjectTooltip` (`project_rail.rs:307-321`).

Smallest seams for R5: the `trailing` match arm `Activity::Done` (`render.rs:949-968`) and the hover swap (`render.rs:1044-1048`); finished-time text for hover lives in the two preview renderers above (or a new `.tooltip`, which would compete with the preview popover on compact rows).

### R7 shell chip "⊡N"

| File:line | What |
|---|---|
| `shell/rails.rs:782-829` | `shell_task_chip(id)`: `shells = &self.agent_status(id).shells`; None if empty; else chip `session-shell-tasks-{id}` with `icon("terminal",10.)` + count, tooltip `CloudTip(commands joined by \n)` |
| `shell/rails.rs:778`, `shell.rs:7007` | Classic/Focused card uses the same helper |
| `agent_status.rs:30-34, 59-63, 151-154` | `AgentStatus.shells: Vec<ShellTask { id, command }>` copied verbatim from `StatusInput.shells`; unrelated to `activity` |
| `conversation.rs:500-509` | `StatusInput.shells` = `self.background.tasks` filtered `kind == TaskKind::Shell` |
| `conversation/background_tasks.rs:13-35` | `BackgroundTasks.tasks` replaced by `set_background(tasks)`; `clear_background()` |
| `conversation.rs:1964-1966` | provider `update.background` -> `set_background` (ignored while stopping) |
| `conversation.rs:2006-2023` | turn `finished` -> `clear_background()` |
| `conversation.rs:3395` | Stop -> `clear_background()` |
| `crates/monocode-core/src/provider.rs:164-175` | `BackgroundTask { id, description, kind, agent, command, started_at }` |
| `crates/monocode-core/src/provider/tasks.rs:620-675` | Claude inventory: `claude_background_list` replaces tracked tasks from `background_tasks_changed`, `remove_claude_task`, `sync_background` publishes `Event::Background(waiting)` where waiting = all tasks once a result was seen, else Shell tasks still in `claude_background` |
| `crates/monocode-core/src/provider/lifecycle.rs:192-221` | after a Claude result with only shells left, turn is `Completed` but the wire stays open; `Event::Done` only when no tasks remain |
| `shell/agent_activity.rs:166-186` | `update_agent_status_for`: input from the loaded chat (`notification_chat`); an unloaded session gets `StatusInput{now,..}` => `shells` empty |
| `shell/agent_activity.rs:179,189` | `terminal_busy(id)` (TUI terminal sessions) only feeds `streaming`, not `shells` |
| `shell/terminal_dock.rs:39,166` | separate "running terminals" concept for the terminal dock (`running_terminals`, foreground process != shell), not per session row |

So the chip is shown whenever the provider's last published background inventory for that loaded chat lists Shell tasks; it disappears when the provider publishes a list without them, the turn finishes (`clear_background`), or the user stops. There is no process-liveness check in the GPUI layer; liveness comes from Claude's `background_tasks_changed` / task notifications (Codex path: `tasks.rs` codex_* and `provider.rs:2119` pushes `Event::Background(Vec::new())`). `agent_status(id).shells` is only as fresh as the last `update_agent_status` for that id.

Smallest seam for R7: `shell_task_chip` (`rails.rs:782-829`) is shared with Classic/Focused; a Tree-only condition belongs at the call `render.rs:1043`.

### Tests pinning behavior

| Test | File:line | Pins |
|---|---|---|
| `classic_and_focused_show_a_chip_for_background_shells` | `shell/rails.rs:1269-1287` | chip visible with `Activity::Idle` + one shell in Classic and Focused (`in_both_session_rails`) — changing `shell_task_chip` itself would affect it |
| `an_opened_unread_row_keeps_its_summary_until_another_session_opens` | `shell/project_tree.rs:1924-1983` | `tree-session-summary-{id}` lingering |
| `a_tree_layout_round_trip_drops_lingering_without_a_fade`, `opening_inbox_demotes_the_finished_row_even_if_the_session_stays_active`, `unread_fades_ease_out_and_the_badge_pops_past_full_size` | `project_tree.rs:2013, 2037, 2070` | unread look/fade |
| archive button | `shell.rs:10660` (`archive-session-1`), `shell/cloud_continue_tests.rs:1156` | Focused/Classic hover archive |

No Tree test asserts the Done caption text or the check glyph.

## Caveats / Not Found

- "Running terminals" for R7: the codebase's per-session count is background shell tasks reported by the provider, not PTY terminals; no per-session "terminal still running" flag exists beyond `shells`.
- The loaded-session preview card uses `summary.updated_at`, not `Done.at`; the unloaded one uses `Done.at`.
- `monocode_core::time::relative` returns compact English ("1h 19m"), no "ago"/"finished" wording.
