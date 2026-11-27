# Research: How Orca tracks and shows agent status

- **Query**: How Orca (multi-agent / worktree desktop app) knows each agent's state, handles background work, shows the current step, tracks unread, aggregates per worktree/project, raises attention (bell, toast, taskbar, sound) and persists state. Map onto Monoptah's planned model.
- **Scope**: mixed (GitHub source, Orca docs, local install, local skill docs)
- **Date**: 2026-11-25

## 0. Product identity (verified)

- It is **stablyai/orca**: MIT, homepage **https://orca.dev**. The homepage is not `onorca.dev`.
  Description: "Orca lets you run a fleet of bots with your own subscription (e.g. Claude Code, Grok, Agy, Codex)".
- Local install: `%LOCALAPPDATA%\Programs\orca\` (Electron). `resources/app-update.yml` has `owner: stablyai, repo: orca`, and `app.asar` `package.json` has version **1.4.272**.
- Source citations below are to `main` @ `6ad780922d8b0151d205663ab32c1b679f31f8a2` (2026-11-25).
  Short form: `orca:<path>` = `https://github.com/stablyai/orca/blob/main/<path>`.
- Local skill docs: `C:\Users\a1103\.rec\claude\accounts\57641\config\skills\orchestration\SKILL.md` is only a stub that runs `orca skills get orchestration`. There is no `orca-cli` folder on disk. The guide is bundled in the CLI (`resources/bin/orca.exe skills get orca-cli`, 375 lines). It only mentions status in passing: `agent list` reports a state, and `terminal wait --for tui-idle`.

## Findings

### Files Found (key ones)

| File | What it does |
|---|---|
| `orca:src/main/agent-hooks/README.md` | Overview: two channels (managed hooks + OSC 7501), precedence, and a per-agent support matrix |
| `orca:src/shared/agent-status/contracts/agent-status-types.ts` | `AgentStatusEntry` / payload: `state ∈ working, blocked, waiting, done`, `workingMode: 'monitoring'`, `toolName`, `toolInput`, `stateStartedAt`, `mainAgent`, `backgroundCommands`, `subagents`, `restoredUnconfirmed` |
| `orca:src/main/claude/hooks/claude-managed-hook-events.ts` | The Claude Code hook events Orca installs |
| `orca:src/shared/agent-hook-listener/providers/claude/claude-events.ts` | Claude event → state mapping |
| `orca:src/shared/agent-hook-listener/providers/claude/background-tasks/claude-background-task-inventory.ts` | Parses `background_tasks` / `session_crons` from Stop/SubagentStop payloads |
| `orca:src/shared/agent-hook-listener/providers/claude/background-tasks/claude-background-task-kind.ts` | Task type → kind: `agent / workflow / command / monitor / unknown` |
| `orca:src/shared/agent-hook-listener/providers/claude/lifecycle/claude-pane-hold-evidence.ts` | What keeps a pane "working" after the lead's turn ends |
| `orca:src/shared/agent-status/lifecycle/agent-lead-status-fold.ts` | `foldAgentLeadStatus`: combines the lead state and child-work liveness into the row state |
| `orca:src/shared/agent-status/child-work/agent-status-child-work-liveness.ts` | Child liveness ranking `waiting > working (agent) > monitoring (non-agent)`, with a fail-active policy |
| `orca:src/shared/agent-status/lifecycle/agent-background-wait.ts` | `agentBackgroundWait` = lead done + monitoring + named background commands |
| `orca:src/shared/agent-status/lifecycle/agent-transitions.ts` | Phases `needs-you / done / working / rest`; transitions; `AGENT_DONE_QUIET_MS = 1500` |
| `orca:src/shared/agent-status/contracts/agent-status-freshness.ts` | `AGENT_STATUS_STALE_AFTER_MS = 30 min`; `AGENT_STATUS_HYDRATE_MAX_AGE_MS = 7 d` |
| `orca:src/shared/agent-status/presentation/agent-row-decay-state.ts` | Stale rows decay to `unverifiable` (live PTY) or `idle` (no PTY); "No update in 34m" label |
| `orca:src/shared/agent-status/presentation/agent-compact-status.ts` | The one-line row text: attention > verdict > "No update" > Stopping > Monitoring > `Tool: input` > message |
| `orca:src/shared/worktree/row-display/worktree-status-rollup.ts` | Worktree dot priority (`rankWorktreeStatus`) |
| `orca:src/renderer/src/components/sidebar/workspace-card/activity/worktree-card-agent-summary.ts` | `SUMMARY_STATE_ORDER` and "N waiting / N working…" count labels |
| `orca:src/renderer/src/attention/agent-attention-acknowledgement.ts` | Unread = `acknowledgedAt < turnStartedAt` |
| `orca:src/renderer/src/attention/agent-auto-ack-targets.ts`, `agent-auto-ack-presence.ts`, `agent-auto-ack-surfaces.ts` | Rules for when a pane counts as "seen" |
| `orca:src/renderer/src/attention/agent-attention-policy.ts` | Whether an event creates unread and/or delivery |
| `orca:src/renderer/src/components/activity/events/useActivityUnreadCount.ts` | The Agents-feed badge count |
| `orca:src/main/notifications/notification-delivery-service.ts` | Gates, cooldown, tray attention, mobile fan-out |
| `orca:src/main/ipc/notifications/native-notification-delivery.ts` | Electron `Notification`; click → reveal window, activate worktree, focus pane |
| `orca:src/main/ipc/notifications/notification-options.ts` | Toast title/body text |
| `orca:src/shared/notifications/notification-settings-defaults.ts` | Default notification settings |
| `orca:src/main/agent-hooks/server/last-status-store.ts`, `server-constants.ts` | Persistence (`last-status.db`) and save cadence |
| `orca:docs/site/content/docs/notifications.mdx`, `activity.mdx` | User docs (bell, Agents feed) |

### 1. How Orca knows an agent's state

**Two channels feed one status store in the main process** (`orca:src/main/agent-hooks/README.md`):

1. **Managed hooks/plugins (primary).** Orca writes hook entries into each agent CLI's config. Each hook script POSTs the hook's stdin JSON to a loopback HTTP server: `http://127.0.0.1:%ORCA_AGENT_HOOK_PORT%/hook/<agent>`, with a token, `paneKey`, `tabId`, `worktreeId` and `launchToken`, all taken from env vars Orca sets in its PTYs. The local copy `~/.orca/agent-hooks/claude-hook.cmd` uses `curl.exe` with a 1.5 s max time, always prints `{}`, and exits early when `CLAUDE_JOB_DIR` is set. That env var marks Claude's background-job daemon workers (`orca:src/main/agent-hooks/installation/scripts/claude-background-job-guard.ts`). There is also a native `resources/bin/orca-agent-hook.exe` (`orca:native/windows-agent-hook`).
2. **OSC 7501 "Program Status Protocol" (fallback).** The agent writes its own status into the PTY stream. Orca parses it and answers the `OSC 7501 ; ?` probe. Claude Code (≥2.1.295), Codex, Grok, pi and Kimi send it. Precedence: hooks win, and OSC 7501 takes over only when hooks are absent, disabled, or **silent for 2 min** while disagreeing (`HOOK_SILENCE_BEFORE_PROGRAM_STATUS_FALLBACK_MS`, `orca:src/main/agent-hooks/server/program-status/hook-precedence.ts`). OSC 7501 also rides along as a facet (progress, a one-line message, sign-in blocks) but never decides state.
3. **Terminal-title heuristic** for agents with neither. It reads the spinner/permission glyphs in the OSC title (`orca:src/shared/agent-status/title/*`, `titleAgentPhase` in `agent-transitions.ts`).

**States.** The wire and stored states are `working | blocked | waiting | done`, plus `workingMode: 'monitoring'`. `blocked` = failure, `waiting` = needs a human. Derived row states add `idle`, `unverifiable`, `interrupted`, `failed` and `unconfirmed` (`orca:src/shared/agent-status/presentation/agent-list-status.ts`). Transition phases: `needs-you` (blocked/waiting), `working`, `done`, and `rest` (a user Stop is not news) (`agent-transitions.ts`).

**Claude Code: hooks installed** (`claude-managed-hook-events.ts`): SessionStart, UserPromptSubmit (async on new versions), Stop, StopFailure, SubagentStart, SubagentStop, TeammateIdle, PreToolUse/PostToolUse/PostToolUseFailure/PermissionRequest (`matcher: '*'`), PostCompact, Elicitation/ElicitationResult, SessionEnd (≥2.1.261), PostToolBatch, plus an optional statusLine.

Mapping (`claude-events.ts`):

- UserPromptSubmit, PreToolUse (not AskUserQuestion), PostToolUse and PostToolUseFailure → `working`.
- PermissionRequest, Elicitation, and PreToolUse(AskUserQuestion) → `waiting`.
- Stop, StopFailure (outcome `failure`, `is_interrupt` → `cancellation`), and a manual PostCompact → `done`.
- A permission wait stays sticky until the same tool-use continues, a new prompt arrives, the child ends, or PostToolBatch settles it (`server-claude-status-rules.ts`).
- Events that carry an `agent_id` update only the subagent roster, except child waits.

**Codex**: SessionStart, UserPromptSubmit, PreToolUse, PermissionRequest, PostToolUse, SubagentStart/Stop, Stop, Interrupt (0.150+), plus a 1 s transcript poll for subagents (`orca:src/main/codex/hooks/definitions/codex-hook-definition.ts`, `server-constants.ts`).

**Gemini**: BeforeAgent, BeforeTool, AfterTool → working; AfterAgent → done. There is no `waiting` from Gemini hooks (`orca:src/shared/agent-hook-listener/providers/gemini/gemini-events.ts`). That approvals then come from the title heuristic is **unverified**.

**Qoder/CodeBuddy** use `Notification` (`permission_prompt` / `elicitation_dialog` → waiting, `idle_prompt` → done) (`compatible-lifecycle-events.ts`).

Amp, command-code, cursor and kiro keep reporting `working` while an approval menu is open (`hook-silent-approval-agents.ts`).

Local note: `C:\Users\a1103\.claude\settings.json` has Orca hooks only for PermissionRequest, Pre/PostToolUse(+Failure), Stop, StopFailure and UserPromptSubmit. That is an older or partial install. The active account config (`.rec\claude\accounts\57641\config\settings.json`) has **no** Orca hooks.

### 2. Background work

- **Source of truth is Claude's own inventory.** Stop/SubagentStop payloads carry `background_tasks[]` (`type`, `status`, `id`, `command`, `description`) and `session_crons[]`. Orca classifies each task by type: `local_agent/subagent/teammate` → agent, `local_workflow` → workflow, `local_bash/shell/background_shell` → command, `monitor` → monitor (`claude-background-task-inventory.ts`, `claude-background-task-kind.ts`). Unknown types and non-terminal statuses **fail active**.
- **Fold** (`agent-lead-status-fold.ts`, `claude-pane-hold-evidence.ts`):
  - the lead's waiting/blocked wins;
  - then a child waiting → `waiting`;
  - then, when the lead is `done`: a live agent child → `working`; only shells, monitors, workflows or crons → `working` + `workingMode: 'monitoring'`; nothing left → `done`.
  - **So "done" does not fire while background shells run.** The row shows "Monitoring background tasks" with a pulse icon (`Activity` from lucide, in `StatusIndicator.tsx`), and `backgroundCommands` lists what runs (`agent-background-wait.ts`).
- **Completion announcement while held** (`claude-held-turn-end.ts`, `heldTurnCountsAsFinished`): if only monitoring holds a turn the user started, Orca stamps `turnCompletedAt` and announces "finished" now. If a subagent holds it, Orca waits for the real `done`. A turn the agent started itself (a task-notification wake-up) is not announced. A failed turn is announced at once.
- **Stale prevention**:
  - Owed task notifications have a **60 s lease** after the lead goes idle (`CLAUDE_OWED_TASK_NOTIFICATION_LEASE_MS`, `claude-owed-notification-expiry-timers.ts`).
  - Silence → after **30 min** a working/waiting row decays to `unverifiable` ("No update in 34m") if its PTY is alive, or to `idle` if not (`agent-row-decay-state.ts`).
  - Restored rows are `restoredUnconfirmed` and never fresh; a sweep retires them when the PTY is provably gone (`restored-status-dead-terminal-sweep.ts`, `unproven-agent-exits.ts`).
  - Child work is retired only by explicit settlement or by session or provider end (`agent-status-child-work-liveness.ts`).
- Long-running dev servers launched as Claude background shells count as `command` → monitoring. A dev server started in a plain terminal is not an agent row; it only makes the worktree `active`.

### 3. Current-step display

- Hooks carry `toolName` (≤60 chars) and a `toolInput` preview. The row line shows `Tool: input` (e.g. `Edit: src/x.ts`), but only while the row is `working` or `waiting` (`agent-row-tool-preview.ts`, `agent-tool-preview.ts`). In monitoring mode it shows "Monitoring background tasks" instead, because the last tool is stale (`agent-compact-status.ts`). The lead line can also come from the OSC 7501 `msg` or the last assistant message.
- Time: the sidebar row shows a **short "time ago" from the done time, else from the start time** (`getCompactAgentTimestamp`, `worktree-card-compact-agent-row.tsx`), plus a prompt-cache countdown (`CacheTimer`).
- A live "Working for 1m 23s / Worked for …" ticker exists in the structured native-chat status (string table in the renderer bundle: `workingFor`, `workedFor`, `failedAfter`, `interruptedAfter`). Whether terminal agent rows show an elapsed ticker as well is **unverified**; in the code read, they show a relative age.
- Worked time accrues only while `working` and not monitoring (`isAgentTimeAccruing`).

### 4. Unread / seen

- Unread is per pane and per turn: `isAgentTurnUnacknowledged(ackAt, turnStartedAt)` = `ackAt < stateStartedAt`. Acks key on `stateStartedAt`, not `updatedAt`, so heartbeats do not re-trigger (`agent-attention-acknowledgement.ts`).
- **"Seen"** = auto-acknowledged when all of these hold (`agent-auto-ack-targets.ts`, `agent-auto-ack-presence.ts`, `agent-auto-ack-surfaces.ts`):
  - the pane is the active tab of a visible split group in the active worktree (terminal view), or in the floating panel;
  - the Orca window is foreground-focused;
  - the user is present: trusted pointer/key/wheel input within `DESKTOP_AWAY_SECONDS = 180`, and the OS idle query does not report away.
  - Rescans run on `visibilitychange` and `focus`.
- If the surface is already viewed when the turn settles, no unread is written, though delivery may still happen (`agent-attention-policy.ts`). "Mark unread" is supported (`manuallyUnreadTurnsByPaneKey`) and lapses on a new turn or subject switch.
- The workspace-level `isUnread` flag (the sidebar dot) is persisted in worktree metadata. It can also be set from the CLI (`orca worktree set --unread/--read`).

### 5. Aggregation

- **Worktree dot priority** (`rankWorktreeStatus`): permission(waiting/blocked) > failed > working > monitoring > unconfirmed > interrupted > done > heuristic (`active` / `inactive`).
- **Summary order** for count labels (`SUMMARY_STATE_ORDER`): waiting, blocked, failed, working, monitoring, unconfirmed, interrupted, done, unverifiable, idle. Labels look like "2 waiting", "1 working", "All 3 agents done".
- Agents-list rank: `AGENT_LIST_STATUS_RANK` (`agent-list-status.ts`).
- The card title is bold with a dot when any agent line, a collapsed child or the workspace itself is unread (`worktree-card-title-status.tsx`).
- **Dimming**: `inactive` = no live PTY, no browser tab, no awake chat and no live agent (`worktree-status.ts` `getWorktreeStatus`, `worktree-activity-state.ts` `isInactiveWorkspace`). Such a card shows a grey dot. A **sleeping** worktree gets `data-worktree-sleeping-dim` (a colour-mix dim), and a disconnected runtime gets `opacity-60` (`worktree-card-surface.tsx`).

### 6. Attention

- **Bell / feed**: the header bell and the "Agents" feed badge count unread historical done/blocked/waiting events plus fresh working turns since the ack. They respect the per-pane "Clear completed" cutoff (`useActivityUnreadCount.ts`, `docs/.../activity.mdx`, `notifications.mdx`).
- **OS notifications**: Electron `Notification`, which on Windows uses the native toast with `setAppUserModelId`.
  - `@orca/windows-notifications` is a native addon that only reads Focus Assist state (`windows-focus-state.ts`).
  - Title: `"<repo> / <worktree> - <Agent> finished|needs input|failed|stopped"`. Body: the last assistant message (180 chars) or `Using <tool>: <input>` (`notification-options.ts`).
  - Clicking: reveal the window → `ui:activateWorktree` → `ui:focusTerminal{tabId, leafId, flashFocusedPane, scrollToBottomIfOutputSinceLastView, ackPaneKeyOnSuccess}`. The click also acknowledges the pane (`native-notification-delivery.ts`).
  - Defaults: enabled, agentTaskComplete and agentNeedsInput on, terminalBell off, `suppressWhenFocused: true` (only when the window is focused **and** the worktree is active), `notifyHiddenWorkspaces: false`.
  - Timing: a 5 s cooldown per worktree/attention key; a 250 ms grace (max 1.5 s) to wait for richer hook detail; a 1.5 s done-quiet window.
  - A notification already read is dropped (`wasRead`).
- **Taskbar**: none found. There is no `flashFrame` (it appears only in the browser-extension API shim), no `setOverlayIcon` and no `setBadgeCount` in the 1.4.272 main bundle.
  - Instead there is **tray attention**: when the window is not visible, an `agent-task-complete` or `terminal-bell` event swaps the tray icon to an attention variant. On macOS the tooltip reads "Orca - activity waiting". Show or restore clears it.
  - The docs claim a macOS Dock badge mirrors the unread count, but the bundle only calls `app.dock.setBadge('')` at startup (**docs likely stale; unverified on macOS**).
  - Whether the Windows tray icon is always present depends on `showMenuBarIcon` (default true) (**unverified**).
- **Sound**: `customSoundId: 'system'` uses the toast's own sound. Otherwise the toast is silent and the app plays a bundled mp3 (`resources/notification-sounds/*.mp3`) or a custom file (≤10 MB) at a set volume. It is skipped while Focus Assist / Focus is active.

### 7. Persistence

- Status rows: SQLite `%APPDATA%\orca\agent-hooks\last-status.db` (legacy `last-status.json`), one row per pane (`last-status-store.ts`).
  - Saves: routine at most every 3 min, turn edges within 1 s, authority changes and quit flush immediately (`server-constants.ts`).
  - On restart, rows hydrate as `restoredUnconfirmed`, are kept up to 7 days, and are never "fresh" until a live hook event confirms them. PTYs survive app restarts through the `orcad` daemon.
- Acks persist in the workspace session UI state: `ui.acknowledgedAgentsByPaneKey`, `ui.activityClearedAtByPaneKey`, `ui.manuallyUnreadTurnsByPaneKey` (seen in the main bundle). The workspace `isUnread` flag lives in worktree metadata. So unread **survives restart**.

### External References

- https://github.com/stablyai/orca — source (MIT)
- https://orca.dev — product site
- https://www.superlogical.com/rex/docs/build/program-status — OSC 7501 spec (linked from the README; not fetched)

### Related Specs

- None in `.trellis/spec/` were consulted for this external survey.

## What Monoptah could borrow

Mapped onto the planned model: **NeedsYou / Running (step + elapsed) / BackgroundWait / Done + unread**, with background shells shown only as a chip.

| Monoptah state | Orca equivalent | Borrowable rule |
|---|---|---|
| **NeedsYou** | `waiting` / `blocked` (phase `needs-you`) | Sources: Claude `PermissionRequest`, `Elicitation`, `PreToolUse(AskUserQuestion)`; Codex `PermissionRequest`; `Notification{permission_prompt, elicitation_dialog}`. Keep a permission wait **sticky** until the same `tool_use_id` resumes, a new prompt arrives, `PostToolBatch` settles it, or the owning child ends. A child's wait outranks lead `working`. Top of every rollup. |
| **Running (step + elapsed)** | `working` (not monitoring) | Step = `toolName: toolInput` from Pre/PostToolUse, shown only while running or waiting. Elapsed starts at `stateStartedAt`, which stays fixed across tool pings; `updatedAt` moves separately. Accrue time only when not monitoring. |
| **BackgroundWait** (non-ambient subagent / workflow / monitor) | lead `done` + `working` (live **agent** child), or `monitoring` | Read Claude's `background_tasks[]` from Stop/SubagentStop and classify by `type`. Orca puts `local_workflow` and `monitor` in **monitoring** together with shells; only `agent` kinds count as "working". The planned model splits non-ambient workflow and monitor into BackgroundWait and leaves shells as a chip. That split is finer than Orca's, but the same `type` field supports it. Fail active on unknown types and non-terminal statuses. Track "owed" task notifications with a lease (Orca uses 60 s). |
| **Background shells: chip only** | `backgroundCommands` + "N shells" copy (`background-task-copy.ts`) | Orca does block "done" on shells (monitoring), but it still announces the turn as finished at once when only monitoring holds a user-started turn (`turnCompletedAt`). That is close to "Done + shell chip". |
| **Done + unread** | `done` + `ackAt < stateStartedAt` | Ack only when the pane is on screen, the window is foreground, and the user was present within 180 s. Key acks on the turn start, not on the last update. Skip the unread write if the pane is viewed at settle time. Persist acks. Use a quiet window (1.5 s) before announcing done. A user Stop goes to `rest`/`interrupted`, not news. |
| Staleness | 30 min → `unverifiable` / `idle`; `restoredUnconfirmed` | Never let a restored or silent row claim Running. Show "No update in Nm" rather than guessing. |
| Attention | Electron toast + tray dot + sound | Toast click → focus the exact pane and ack it. Cooldown per worktree (5 s). Suppress only when the window is focused **and** the worktree is active. Orca has **no** taskbar flash or overlay badge, so Monoptah's planned taskbar badge or flash goes beyond what Orca does. |

## Caveats / Not Found

- The Gemini approval path (no `waiting` hook) is unverified; the title heuristic is assumed.
- Whether terminal agent rows show a live elapsed ticker, as opposed to a relative "time ago", is unverified.
- The macOS Dock badge (claimed in the docs) was not found in the 1.4.272 bundle.
- No Windows taskbar flash or overlay was found. Whether the Windows tray icon shows by default is not verified.
- The exact Claude Code version that started emitting `background_tasks` / `session_crons` in hook payloads was not checked. Orca treats an absent inventory as "keep the roster".
- Files were read from `main` on 2026-11-25. The local build is 1.4.272 and may differ slightly from `main`.
