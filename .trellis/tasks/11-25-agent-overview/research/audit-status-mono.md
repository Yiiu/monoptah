# Research: audit of status core and Mono phases 1–2 against their requirements

- **Query**: For every requirement bullet and acceptance criterion in `11-25-agent-status-core` (R1–R5, R6 deferred), `11-25-mono-dispatch-visibility`, `11-25-mono-patrol`, and the Mono and cross-child sections of `11-25-agent-overview/prd.md`, classify each one as DONE / PARTIAL / MISSING / CHANGED, with file:line evidence.
- **Scope**: internal (read-only)
- **Date**: 2026-11-26
- **Tree audited**: `.worktrees/agent-status-core`, branch `feat/agent-overview-tree` (PR #22), commit `a2841072` (implementation) plus `4f4a1fd5` (task docs). Base for diffs: `22714cf7`.
- Paths below are relative to `crates/`. `core` = `monocode-core/src`, `gpui` = `monocode-gpui/src`.

Legend: **DONE** = built as specified. **PARTIAL** = some of it is built; the entry says what is missing. **MISSING** = not built. **CHANGED** = built a different way from the spec; the entry says how.

---

## 1. Status core (`11-25-agent-status-core`)

### R1 Unified status model

| Item | Class | Evidence |
|---|---|---|
| One pure `derive()` in core, with no GPUI | DONE | `core/agent_status.rs:85-131` |
| Precedence NeedsYou > Running > BackgroundWait > Done (then Idle) | DONE | `core/agent_status.rs:95-126`; test `needs_you_then_running_then_wake_work_then_usage_limit_then_done` (`:221`) |
| NeedsYou covers approvals, questions and remote waiting | DONE | `core/agent_status.rs:86-94`; remote input set at `gpui/shell/agent_activity.rs:187` |
| Running carries the step (last tool call) and `since` | DONE | step from `TranscriptView::live_turn` at `gpui/conversation.rs:470-516` (`step:` near `:508`); `since` is kept per session in `Timing` and resets only when the variant changes (`gpui/shell/agent_activity.rs:280-292`) |
| Terminal and remote sessions get no step, only the time | DONE | `step: (streaming && step && !self.is_remote())` in `gpui/conversation.rs` `agent_status_input`; terminal sessions take the default input (`agent_activity.rs:180-188`) |
| BackgroundWait for non-ambient subagent, workflow or monitor work | DONE | `wake_tasks` comes from `TurnActivity::Yielded` (`gpui/conversation.rs` `agent_status_input`); ambient tasks are dropped (`core/provider/tasks.rs:~486`, `:631-660`) |
| BackgroundWait for a usage limit with auto-resume | DONE, and CHANGED | `Wait::UsageLimit{resume_at}` and an added `Wait::UsageLimitPending` (reset time unknown) at `core/agent_status.rs:11-15,108-117`. The source is `auto_resume_state` (`gpui/conversation.rs` `agent_status_input`). The pending variant is not in the design. |
| Shell tasks never affect status and are output as their own list | DONE | `core/agent_status.rs:127-130`; test `shell_inventory_never_holds_a_turn_open` (`:300`) |
| Done outcomes: Completed, Failed, Interrupted (Stopped) | DONE | `core/agent_status.rs:17-22`. Outcome assignment: Stop gives Interrupted (`gpui/conversation.rs:3371`); runtime finish gives Interrupted if stopping, Completed if status is "Ready", otherwise Failed (`:2002-2008`); a Claude logical completion gives Completed (`:~1965`); a TUI exit code maps to an outcome (`gpui/shell/tui_sessions.rs:189-199`); a killed TUI pane gives Interrupted (`:177-181`) |
| Idle state for drafts and never-run sessions | DONE, and CHANGED (additive) | `Activity::Idle` (`core/agent_status.rs:55-56`) is in the design, not in the PRD table |

### R2 Background task correctness (Claude)

| Item | Class | Evidence |
|---|---|---|
| A shell-only background turn ends at `result` and goes to Done | DONE, and CHANGED (T1 fallback) | `sync_claude_result` (`core/provider/lifecycle.rs:95-124`) emits `TurnActivity::Completed` while the wire stays open; when the wire closes, `Event::Done` follows (`core/provider.rs:2112-2121`). GPUI treats Completed as the end of the turn (`gpui/conversation.rs` activity match, around `:1961-1984`). T1 is recorded in `11-25-agent-status-core/research/state-plumbing.md:247-265` as "status-only fallback", using an explicit `TurnActivity` event rather than an auto-started turn. Test: `claude_shell_only_result_is_complete_and_still_receives_its_wake` (`core/provider/tasks.rs:1078`) |
| `TaskKind {Shell, Agent, Monitor, Workflow, Other}` with `Other` treated as a wake task | DONE | `core/provider.rs:147-153`, `classify_task` at `core/provider/tasks.rs:698-707`; only `Shell` is excluded from wake (`lifecycle.rs:100-106`) |
| The `background_tasks_changed` list is authoritative and prunes everything | DONE | `claude_background_list` (`core/provider/tasks.rs:631-662`) removes missing ids through `remove_claude_task` (`:620-629`) and replaces `claude_background`. `claude_pending` is deleted entirely. Test: `claude_does_not_wait_for_a_task_missing_from_the_full_inventory` (`:1041`) |
| A terminal `task_updated` or `task_notification` removes the id | DONE | `core/provider/tasks.rs:479-497` |
| When a background task ends and Claude wakes, status returns to Running | DONE | `starts_message && claude_result_seen` resets the turn and emits `TurnActivity::Running` (`core/provider.rs:536-554`); GPUI re-opens streaming on `woke` (`gpui/conversation.rs:~1844-1852`). Tests: `claude_waits_for_the_lead_to_wake_and_finishes_after_the_continuation` (`tasks.rs:1135`) and the shell wake test (`:1078`) |
| Background changes reach the UI immediately | DONE | `background_changed` and `activity_changed` emit `SessionChanged` (`gpui/conversation.rs:1827,2085`); the provider flushes activity edges at once (`core/provider.rs:~1982-1990, 2085-2092`) |
| Codex unchanged (no background) | DONE | `agent_turn == None` gives `streaming = responding()` (`gpui/conversation.rs` `agent_status_input`) |
| Hermes maps to `wake_tasks = acp.background.len()` | DONE, with no dedicated test | `core/provider/acp.rs:920-922` (Yielded), `:1116` (Running on the handoff re-prompt). No test asserts `TurnActivity` for ACP. |

### R3 Read / unread

| Item | Class | Evidence |
|---|---|---|
| Seen requires the session to be visible, the window active, and input within 180 s | DONE | `PRESENCE_MS = 180_000` (`gpui/shell/agent_activity.rs:9`); the gate is at `:237-265` and `:347-377`; visibility, including splits and the Mono routed panel, is at `gpui/shell/notifications.rs:442-465`; input listeners (mouse move, key down, scroll, mouse down) at `gpui/shell.rs:9934-9950`; window activation at `:9760-9764`; visibility sync at `:1773`. Test: `working_marks_animate_independently_and_completed_replies_clear_when_seen` (`gpui/shell.rs:10623`) checks that a session idle for more than 180 s stays unread |
| A notification click counts as seen | DONE, with no test | `this.mark_agent_seen(id, cx)` at `gpui/shell/notifications.rs:695` (Mono) and `:747` (session). No test found. |
| Read state survives a restart | DONE | `seen_at` in `session_activity` (`core/store/session_activity.rs:224-241`); restored at `gpui/shell/agent_activity.rs:81-96` and `gpui/main.rs:518-523`. Test: `out_of_order_turn_writes_and_acknowledgements_keep_the_latest_completion_unread` (`session_activity_tests.rs:118`) |
| The first launch after upgrade marks every existing session read | DONE, and CHANGED | The migration inserts `turn_ended_at = updated_at` (capped at now) and `seen_at = now`. Drafts get no turn, so they show Idle instead of a read Done (`core/store/session_activity.rs:34-62`). The design said `turn_ended_at = seen_at = now`. Test: `migration_marks_existing_history_read_once_and_leaves_drafts_idle` (`session_activity_tests.rs:64`) |
| `Shell.unseen_finished` removed | DONE | No occurrences remain in `gpui/` |

### R4 Anti-stuck

| Item | Class | Evidence |
|---|---|---|
| After a restart, a turn whose process is gone shows Done (Stopped) | DONE, and CHANGED | The owner is the Monoptah process (pid plus start stamp, written by `begin_session_turn`, `core/store/session_activity.rs:119-141`), not `in_flight_sessions`. The sweep is in `recover_session_turns` (`:331-382`) and sets `outcome='interrupted'` and `progress_summary='Stopped before finishing'`, unread. It runs from `restore_session_activity` at startup (`gpui/main.rs:518`). Local only; remote and cloud sessions are skipped (`:360`). Test: `restart_recovers_only_orphaned_local_turns_and_keeps_the_result_unread` (`session_activity_tests.rs:141`) |
| A Running session with no event for 30 minutes shows "No update in 30m" and stops spinning | PARTIAL | Rule at `core/agent_status.rs:83,101,144`; `is_animating` is false when stale (`:134-136`). Chats update `agent_event_at` on every provider batch or row (`gpui/conversation.rs:~1830-1836`). **Gap:** terminal (TUI) sessions have no event source. `TuiEvent::Busy` fires only on busy transitions (`gpui/tui_session.rs:437-442`), and `last_event_at` falls back to the turn start or `timing.since` (`gpui/shell/agent_activity.rs:270-278`). A TUI turn that keeps producing output for more than 30 minutes therefore shows "No update in 30m". Test: `running_uses_last_event_for_staleness_and_since_for_elapsed` (core only) |

### R5 Existing layouts use the new status

| Item | Class | Evidence |
|---|---|---|
| Running: "step · 4m" or "Working · 4m", refreshed about every 15 s | DONE, and CHANGED | The label is at `core/agent_status.rs:145-148`. `STATUS_TICK = 15s` (`gpui/shell/live_agents.rs:8,86-110`) calls `refresh_status_clocks` (`agent_activity.rs:124-143`). Change: the step text is also recomputed on every `AgentProgress` event (`gpui/conversation.rs:2089`, subscribed at `gpui/shell/notifications.rs:721`), not throttled to 15 s as the design said. |
| BackgroundWait: "Waiting on N subtasks" and "Usage limit · resumes in 23m" | DONE | `core/agent_status.rs:149-170`. Rows show the usage label through `auto_resume_activity_badge` (`gpui/shell.rs:7479-7522`), with the label shown only when the row is detailed |
| Done: "Done", "Failed" or "Stopped" while unread; relative time once read | DONE | `core/agent_status.rs:171-181`; relative time at `gpui/shell.rs:~7466-7474` and `gpui/shell/rails.rs:~764-768` |
| A small chip for shell tasks, with commands in the tooltip | DONE | `shell_task_chip` (`gpui/shell/rails.rs:782-810`), used in Classic (`rails.rs:778`), Focused (`gpui/shell.rs:7159`) and Tree (`gpui/shell/project_tree/render.rs:820`) |
| Classic and Focused rows show the new labels | DONE, with no row-level GPUI test | Focused `session_suffix` (`gpui/shell.rs:7407-7476`); Classic `classic_session_status` (`gpui/shell/rails.rs:732-780`). No GPUI test asserts the row text "Waiting on", "Failed", "Stopped", "Usage limit" or the shell chip; the core tests cover only "Running tests · 4m", "No update in 30m" and "Done" |
| UI copy is in English | DONE | Every label in `core/agent_status.rs:138-183` |

### Consumers reading the one cache (`Shell::agent_status`, `gpui/shell/agent_activity.rs:99`)

| Consumer | Class | Evidence |
|---|---|---|
| Sidebar rows (Classic, Focused, Tree) | DONE | `rails.rs:737`, `shell.rs:7428-7468`, `project_tree/render.rs:699` |
| Tabs and session marks (with the new `Waiting` and `Stale` states) | DONE | `MarkState::from_activity` (`gpui/session_mark.rs:64-73`); marks updated at `agent_activity.rs:293-304`. Test: `mark_spins_only_while_the_agent_is_actively_running` (`session_mark.rs:442`) |
| Filters | DONE | `gpui/shell/session_filters.rs:108-121`. Tests at `:131` and `:150` |
| Folder rows and hover previews | DONE | `session_groups.rs:1578,1763-1766,1950-1965`; `Conversation::preview(status)` (`gpui/conversation.rs`, `classic_sidebar.rs:961-964`, `shell.rs:7928-7931`) |
| Live agents card | DONE | `gpui/shell/live_agents.rs:43-85`. Test: `activity_visibility_switch_preserves_agents_and_survives_restart` (`sidebar_list.rs:1269`) |
| Notifications (end of turn, Dock badge) | DONE | The end-of-turn notice moved to `notification_turn_finished` (`gpui/shell/notifications.rs:115`), triggered from `agent_activity.rs:335-339`. The badge counts `needs_you()` from the cache (`notifications.rs:412,498`) |
| Mono panels and cards | DONE | `mono_status` reads `displayed_activity` (`gpui/conversation/mono.rs:400-407`); `MonoSession.activity` and `needs_input` (`gpui/shell/monos.rs:114,118`); panels (`gpui/monos/panels.rs` `AgentState::Activity`); cards (`gpui/monos/cards.rs:321,337`) |
| Inbox running counts | DONE | `push_inbox_busy` (`gpui/shell.rs:2049-2059`) from `busy_sessions` (Running or BackgroundWait) |
| Today | DONE | `gpui/shell/today_sessions.rs:21` |
| Mobile catalog and watch, wire format unchanged | DONE | `mobile_agent_waiting` and `mobile_waiting_label` (`gpui/shell/mobile_host.rs:86-102`); catalog `:764-787`; snapshots `:1823-1833,3519-3540`. Keys stay `busy` and `waiting` (`gpui/mobile/catalog.rs:134-144`). Non-live entries now get both values from the cache (`mobile_host.rs:~971-976`). No test targets the new mapping |
| Remaining direct readers (not display) | NOTE | `gpui/shell/rails.rs:1054` (`responding()` gates the Stop button on orchestrator workers); `gpui/shell/app_commands.rs:486` reports `"busy": metadata.busy` to the Mono and Operator `sessions` tool, so a session holding only a dev server still reports busy there; `monos/scheduler.rs:358,374` (habit approval timeout). These are behaviour or tool gates, not UI |

### R6 Windows alerts (deferred)

| Item | Class | Evidence |
|---|---|---|
| Nothing half-built or left dead | DONE (correctly absent) | No `platform_windows`, `set_app_identity`, `SetOverlayIcon`, `FlashWindowEx` or `PlaySoundW` anywhere in `gpui/`. No changes to `Cargo.toml`, `Cargo.lock`, `gpui/notifications/` or `sound.rs`. The `cfg_attr` dead-code gates are unchanged. The only Windows code added is `process_owner` in `core/store/session_activity.rs:444-490`, which uses features `windows-sys` already enables (`monocode-core/Cargo.toml:65`) |

### Status-core acceptance criteria

| AC | Class | Notes |
|---|---|---|
| A dev-server-only Claude session shows Done plus the shell chip, and the server keeps running | DONE (automated in core; manual run not recorded) | Core test at `tasks.rs:1078`; the chip is rendered in all three layouts |
| A subagent shows "Waiting on N subtasks", then Running on wake, then Done | DONE | Test `tasks.rs:1135`, label `agent_status.rs:149-155` |
| A task that vanishes from the list does not leave the session stuck | DONE | Test `tasks.rs:1041` |
| Every view agrees on one session's status at the same moment | DONE | Single cache; see the consumer table |
| Unread survives a restart; the first launch after upgrade has no flood of unread marks | DONE | Migration test `session_activity_tests.rs:64` |
| Window active but the user away for more than 3 minutes leaves the session unread | DONE | `shell.rs:10623` |
| After a restart, a dead turn shows Stopped, not Working | DONE | `session_activity_tests.rs:141` |
| No new settings; macOS notification behaviour unchanged | DONE, with a note | No new setting found. The finished-notice path was rewritten: `NotificationFinished` became `HabitFinished`, and the exclusion moved from `is_mono_session` to habit sessions only (`gpui/shell/notifications.rs:115-136`). The intended behaviour looks the same, but no test covers it |
| Linux CI and Windows `cargo check` pass | DONE | `gh pr checks 22`: every job passes, including GPUI tests on 3 shards, GPUI lint, GPUI check on Windows and Core on Windows. The overview PRD still says "Linux CI not yet verified" (`11-25-agent-overview/prd.md`, "实现与验证"), which is stale. The same section names branch `fix/agent-status-core`, while the code is on `feat/agent-overview-tree` |
| Manual run in both layouts (implement step 7) | MISSING (not recorded) | The overview PRD says desktop visual checks and real providers are not yet verified |

---

## 2. Mono phase 1 (`11-25-mono-dispatch-visibility`, and the Mono section of the overview PRD)

| Item | Class | Evidence |
|---|---|---|
| Each card records its own answer (the old "one answer disables all cards" bug) | DONE | Records keyed by card id: `Mono.decision_cards` (`core/mono.rs:72`), `DecisionRecord` (`core/mono/cards.rs:6-13`), `answer_decision` (`gpui/monos/cards.rs:447-486`). The old `reply_after` was removed and choices now render through `mono_decision` (`gpui/transcript.rs`, the `Body::MonoCard` arm). Test: `decision_cards_keep_independent_durable_answers` (`gpui/monos/tests.rs:17`) |
| Multi-select, "Other" free text, and more than 4 options | DONE, and CHANGED | `multiple: true` and `allow_custom: true` on every card (`core/mono/cards.rs:44-48`); the inline `QuestionCard` placeholder is "Other" (`gpui/questions.rs` `with_mode`). The 1–4 limit was removed with **no upper bound** (`core/mono/cards.rs:129-134`). Labels must be distinct (`:171-173`). Tests: `decisions_support_more_than_four_options_and_custom_multiple_answers` (`core/mono/cards.rs:240`) and `cards_validate_fields…` |
| Old-format cards are compatible: an old reply resolves one matching card | DONE | `legacy_records` (`gpui/transcript/mono_decisions.rs:21-77`). Test: `legacy_choice_replies_resolve_only_one_matching_card` (`:200`) |
| Answers and unanswered state survive a restart | DONE | The answer and its queued message are saved in one roster save (`gpui/monos/cards.rs:481-484`); pending chats are restored at startup (`gpui/monos/mod.rs:~575-600`). Restart is covered in the `monos/tests.rs:17` test |
| The Mono prompt tells it to use cards | DONE | `core/mono.rs:470`; bridge declaration at `core/mono/bridge.rs:80`; heartbeat wording at `core/mono/heartbeat.rs:~418` |
| Unanswered decisions enter the unified status and the Needs-you queue | DONE, with a risk | `questions` includes `pending_mono_decisions` (`gpui/conversation.rs:490`); notification inputs at `gpui/conversation/notifications.rs:26-34`; the Tree queue includes Mono chats (`gpui/shell/project_tree.rs:556-572`). **Risk:** pending decisions also include *legacy* cards with no recorded answer (`mono_decisions.rs:113-124`). Any pre-upgrade choice card that was followed by an unrelated message stays unanswered (`:44`), so after the upgrade an existing Mono can show NeedsYou for old cards in its loaded transcript tail |
| Mono-dispatched sessions appear in the Tree by default, under their worktree | DONE, with no test | `listed = launched_by_mono || …` (`gpui/shell/project_tree.rs:495-500`); `session_worktrees` is filled for hidden launches (`gpui/shell/monos.rs:~143-170`; `mono_track_sessions` is deferred from `gpui/monos/mod.rs:~602-610`). No test asserts it |
| The Mono pixel mascot follows the title | DONE, with no test | `gpui/shell/project_tree/render.rs:782-787,816` (10px avatar, state "idle") |
| Classic and Focused keep their own visibility settings | DONE | `sidebar_session_ids` is unchanged in its use of `hidden_session` (`gpui/shell.rs:~7535-7545`) |
| The Mono overview plan checklist ("N done · N running · N needs you") comes from its sessions | DONE, with no test | `plan()` at `gpui/monos/dashboard.rs:274-460`, label at `:335`, fed by `MonoSession.activity` from the cache; inserted at `:~994`. No test for the plan or its counts |
| Verification of core parsing, GPUI card isolation and all-target build | DONE | Tests above; CI passes |

## 3. Mono phase 2 patrol (`11-25-mono-patrol`)

| Item | Class | Evidence |
|---|---|---|
| Periodic patrol of the Mono's projects (agents and worktrees), using the unified status and progress summaries | DONE | Sessions are collected in `heartbeat_candidates` (`gpui/monos/scheduler.rs:1485-1545`; state via `From<&Activity>` at `core/mono/patrol.rs:40-54`; summary from `shell.progress_summary`). Worktrees come from `patrol_inventory` (`scheduler.rs:40-81`: `git::worktrees` and `scm::diff_stats`). It runs on the existing 10 s Mono clock (`gpui/monos/mod.rs:562-572`) and only when `heartbeat::due` passes (`scheduler.rs:~111`) |
| Cloud sessions are grouped under their local project | DONE | Test `patrol_groups_cloud_sessions_with_their_local_project` (`scheduler.rs:2159`) |
| The baseline persists across restarts; the first patrol only seeds it | DONE | `heartbeat::State.patrol` and `pending_patrol` (`core/mono/heartbeat.rs:~32-35`), seeded at `scheduler.rs:93-102`. Test: `patrol_first_seeds_then_retries_changes_until_confirmed` (`heartbeat.rs:550`) |
| Silent when nothing changed (no model call) | DONE, with a caveat | `if digest.is_empty() && changes.is_empty() && standing.is_none() { return Ok(None) }` (`scheduler.rs:~168`). Tests: `patrol_uses_default_interval_and_only_briefs_project_changes` (`:2174`) and `heartbeat_seeds_waits_for_quiet_then_raises_a_left_session_once` (`:2105`). Caveat: a Mono with standing instructions in its soul still makes a model call every interval. The design keeps this ("已有用户明示的定期心跳检查继续使用原有调度") |
| Seen-state flips and clocks are not reported | DONE | `State` drops `since`, `stale` and `unread`; test `patrol_ignores_presence_and_display_clocks` (`core/mono/patrol.rs:167`) |
| Summarized progress: changes go into the prompt as JSON and the Mono summarizes | DONE | `patrol::briefing` (`core/mono/patrol.rs:148-160`) is appended to the heartbeat message (`scheduler.rs:~170-181`) |
| At most 20 changes per batch, NeedsYou first; the rest roll over | DONE | `observe_patrol` (`core/mono/heartbeat.rs:~64-88`). Test: `a_large_patrol_reports_urgent_changes_and_defers_unreported_changes` (`:575`) |
| The batch is confirmed only after a successful run with its report saved; failures retry | DONE | `confirm_patrol` is called only when `ok && published` (`scheduler.rs:~2030-2050`) |
| Old heartbeat records still load | DONE (by `serde(default)`) | The new fields are `Option` with `skip_serializing_if`. The test deserializes a round-trip, not a literal old JSON file |
| Reuses the existing heartbeat schedule and quiet rules | DONE | `heartbeat::due` and `interval_ms` are unchanged |
| No new personal settings | DONE | No new setting keys. The Tree layout is a third value of the existing Layout picker (`gpui/settings/pages/appearance.rs`, `gpui/workspace.rs` `NavigationLayout`), as the parent PRD requires. `tree_session_order` is persisted view state, not a setting |
| Monos without a chat session can now patrol | CHANGED (not in spec) | The candidate filter is `session_id.is_some() || !projects.is_empty()` (`scheduler.rs:~1499`); `start_run` builds a session from the default provider or model when no chat exists (`:~1790-1825`) |

## 4. Cross-child acceptance criteria (`11-25-agent-overview/prd.md`)

| AC | Class | Notes |
|---|---|---|
| A dev-server-only session no longer shows Working; a vanished task does not leave status stuck | DONE | See R2 |
| One status across sidebar, project counts, Needs-you queue, notifications and the Mono plan | DONE | Single cache; project counts at `project_tree.rs:540-553` |
| No new personal settings | DONE | See above |
| Classic and Focused behaviour unchanged; Tree is a third option from settings, menu and shortcut | DONE (layout part is outside this audit's depth) | `NavigationLayout::next` (`gpui/workspace.rs`); test `navigation_cycles_and_restores_existing_layouts`. Side effect: the appearance description text now describes Tree only |
| Mono mascots and their animations visible in Tree | DONE (roster not re-audited) | Launch mascot at `render.rs:782-787` |
| Feature parity in Tree | Not audited here | Belongs to `11-25-project-tree-sidebar` |

---

## (a) Tests by area

- **Status model (core)**: `agent_status.rs` – `needs_you_then_running_then_wake_work_then_usage_limit_then_done`, `shell_inventory_never_holds_a_turn_open`, `running_uses_last_event_for_staleness_and_since_for_elapsed`. Not covered: the labels "Failed", "Stopped", "Waiting on N", "Usage limit · …", and `UsageLimitPending`.
- **Claude adapter**: `provider/tasks.rs` – `claude_reports_shells_during_the_answer_and_subagents_once_it_yields`, `claude_does_not_wait_for_a_task_missing_from_the_full_inventory`, `claude_shell_only_result_is_complete_and_still_receives_its_wake`, `claude_waits_for_the_lead_to_wake_and_finishes_after_the_continuation`. Also `provider/follow_up.rs` – `claude_replayed_task_notification_result_does_not_end_the_turn` (`:438`), `claude_turn_with_a_real_task_notification_run_still_ends` (`:471`), `claude_turn_ends_when_a_queued_prompt_never_starts` (`:481`).
- **Hermes and ACP mapping**: none.
- **Persistence**: `store/session_activity_tests.rs` – `summaries_only_change_the_matching_finished_turn`, `restoring_a_session_keeps_its_consumed_worktree_event`, `migration_marks_existing_history_read_once_and_leaves_drafts_idle`, `out_of_order_turn_writes_and_acknowledgements_keep_the_latest_completion_unread`, `restart_recovers_only_orphaned_local_turns_and_keeps_the_result_unread`, `deleting_a_session_clears_its_activity_even_when_prototype_ids_are_reused`.
- **Seen rule**: `shell.rs` – `working_marks_animate_independently_and_completed_replies_clear_when_seen`. Notification-click seen: none.
- **Consumers**: `session_mark.rs` – `mark_spins_only_while_the_agent_is_actively_running`, `running_outline_keeps_a_uniform_gap_and_loops_at_the_same_point`; `session_filters.rs` – `working_filter_excludes_a_remote_session_that_needs_input`, `filters_persist_and_update_without_closing_the_menu_or_active_chat`; `tui_sessions/tests.rs` – `busy_marks_the_session_and_the_cli_exiting_clears_it`, `killing_the_pane_clears_a_busy_mark`; `sidebar_list.rs` – `activity_visibility_switch_preserves_agents_and_survives_restart`; `project_tree.rs` – `collapsed_projects_still_count_waiting_work_but_exclude_archive`, `needs_you_opens_the_other_project_and_uses_its_worktree`, `needs_you_iteration_wraps_and_recovers_if_a_request_has_resolved`, `an_opened_unread_row_keeps_its_summary_until_another_session_opens`. Not covered: mobile catalog mapping, Inbox counts, Today, Classic and Focused row text, and the shell chip.
- **Mono phase 1**: `core/mono/cards.rs` – `decisions_support_more_than_four_options_and_custom_multiple_answers`, `cards_validate_fields_and_do_not_accept_arbitrary_links_or_commands`; `gpui/transcript/mono_decisions.rs` – `legacy_choice_replies_resolve_only_one_matching_card`; `gpui/monos/tests.rs` – `decision_cards_keep_independent_durable_answers`. Not covered: Tree Mono listing and mascot, and the plan checklist.
- **Mono phase 2**: `core/mono/heartbeat.rs` – `patrol_first_seeds_then_retries_changes_until_confirmed`, `a_large_patrol_reports_urgent_changes_and_defers_unreported_changes`; `core/mono/patrol.rs` – `patrol_ignores_presence_and_display_clocks`; `gpui/monos/scheduler.rs` – `heartbeat_seeds_waits_for_quiet_then_raises_a_left_session_once`, `patrol_groups_cloud_sessions_with_their_local_project`, `patrol_uses_default_interval_and_only_briefs_project_changes`; `gpui/monos/tests.rs` – `heartbeat_reads_inactive_monos_saved_provider_and_permissions`.

## (b) Areas that need a separate review

1. **Migration** (`core/store/session_activity.rs:25-115`): it creates the table in the `*.gpui.sqlite3` sidecar (`native_db`, `core/store/shared.rs:370-375`; it falls back to `self.conn` only when there is no sidecar, which means in-memory or read-only stores). It runs once in an IMMEDIATE transaction. The `ALTER TABLE` steps add `progress_summary` and `auto_archive_key`. The writer re-runs `restore_session_activity` if it sees "no such table" (`gpui/shell/agent_activity.rs:54-61`); that re-run would mark everything read again, and its result is discarded, so the in-memory map is not refreshed.
2. **Restart sweep and data loss**: ownership is the app's pid plus a start stamp (`session_activity.rs:119-141,414-499`). On Windows, any `OpenProcess` failure other than `ERROR_INVALID_PARAMETER` counts as Alive, so the turn is never swept. The sweep overwrites `progress_summary` with "Stopped before finishing". Deleting a session clears its row (`core/store.rs:~1521,1543`). Auto-archiving after a merge or worktree cleanup (`gpui/shell/progress_summary.rs:191-330`, part of the 2b task) archives sessions without user action and should get its own data-loss review.
3. **Main-thread CPU and timers**:
   - Every provider batch emits `AgentProgress` (`gpui/conversation.rs:2089`). That runs `update_agent_status_for`, which calls `TranscriptView::live_turn` (an `rposition` over `source` plus a JSON parse of the last tool call, `gpui/transcript.rs:1085-1120`), roughly every 16 ms while a session streams.
   - The 15 s status tick (`live_agents.rs:86-110`) also runs for the whole length of a usage-limit wait (`agent_activity.rs:111-122`).
   - The 10 s Mono clock now builds `mono_session_views` over every session for every Mono in `heartbeat_candidates` (`scheduler.rs:1485-1545`), even when no heartbeat is due.
   - When a heartbeat is due, `patrol_inventory` runs `git worktree list` plus `diff_stats` per worktree on a background thread.
   - The worktree-archive watcher runs git every 75 s in Tree layout (`progress_summary.rs:175,191-205`).
   - `refresh_legacy_mono_decisions` re-scans and re-parses every Mono-card row on each transcript rebuild (`gpui/transcript.rs:~3418`).
4. **Unbounded growth**: `Mono.decision_cards` is never pruned (`core/mono.rs:72`). The patrol snapshot in `heartbeat.json` keeps every non-skipped session of the Mono's projects, archived ones included (`scheduler.rs:~1505-1530`). Choice cards have no upper limit on the number of options (`core/mono/cards.rs:129-134`).

## (c) TODO and FIXME in the new files

None. `TODO`, `FIXME`, `XXX`, `HACK`, `todo!` and `unimplemented!` were searched for in `core/agent_status.rs`, `core/store/session_activity.rs`, `core/store/session_activity_tests.rs`, `core/mono/patrol.rs`, `core/response/progress.rs`, `gpui/shell/agent_activity.rs`, `gpui/shell/progress_summary.rs` and `gpui/transcript/mono_decisions.rs`.

## Caveats / not found

- Line numbers marked `~` are approximate, taken from diff hunks. All others were read directly.
- The Tree-layout feature-parity checks (`11-25-project-tree-sidebar`) and the progress-summary and archive requirements (`11-25-progress-summary`) were not audited in depth.
- Runtime behaviour was not executed. This classification comes from reading the code and from CI status.
