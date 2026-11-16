# Monoptah Native `/goal` Support — Design

Date: 2026-11-16
Status: builder check done (2026-11-16); all four findings resolved by
investigation or controller call, none left to the design owner
Scope: Monoptah Native (GPUI) only. The Tauri app is explicitly out of scope.

## Intent and why

Claude Code 2.1.x ships `/goal`: a persistent session goal enforced by a
session-scoped Stop hook that blocks turn-end until the model meets the
condition, with `/goal clear` to cancel. The feature works at the wire level
under Monoptah Native's exact spawn flags (verified 2026-11-16 against
claude 2.1.311: the stream-json `system/init` advertises `goal` among 78
slash commands, and a user message beginning with `/goal …` is executed by
the CLI, Stop-hook gating included).

Two app-layer defects keep users from reaching it:

1. **Preamble injection breaks command execution.** Every plain local claude
   turn gets the terminals preamble (and operator/today preambles where
   applicable) prepended to the user's prompt
   (`crates/monocode-gpui/src/runtime.rs:606-668`, gated at `:637-648`).
   The CLI only executes a slash command when the message *starts* with `/`,
   so `/goal …` degrades to ordinary prose. The existing raw-passthrough
   carve-out covers only OMP and Devin (`runtime.rs:489-492`), whose comment
   names this exact failure mode.
2. **Two composer paths swallow Enter on slash input.**
   - Main composer: with a `/token` completion active, Enter either accepts
     a completion or (zero choices) merely dismisses the picker — it never
     submits (`crates/monocode-gpui/src/composer_completion.rs:1456-1465`).
   - Quick Composer: a leading `/word` opens a picker over a static 4-entry
     mode-command list; Enter routes to `choose_command`, which returns
     silently on no match (`crates/monocode-gpui/src/quick_composer.rs:439-443`,
     `638-641`). `/goal` matches nothing there.

Users experience this as "/goal does nothing in Monoptah".

## Goals

1. `/goal <condition>` typed in the main composer executes in the CLI and
   visibly takes effect in the app.
2. The session shows persistent goal state and offers one-click clear.
3. The passthrough fix is generic: any command the CLI advertises in
   `slash_commands` becomes executable (`/model`, `/status`, … for free).
4. Both Enter-swallowing paths submit instead of going quiet.
5. Goal state survives session resume.

## Non-goals

- No app-side goal evaluation. The CLI's Stop hook owns gating.
- No model-proposed goals (ProposeGoal): the CLI errors outside its
  interactive TUI; stream-json sessions cannot use it.
- No Tauri changes.
- No sidebar goal indicator (cut to keep scope tight).
- No special picker entry for goal — it already arrives via the provider
  command catalog once commands actually execute.

## Decisions

### 1. Raw passthrough for advertised commands (generic, not goal-specific)

At turn launch in `runtime.rs`, treat a prompt as a raw native command when:
the provider is Claude, the trimmed prompt starts with `/`, and the first
word (minus `/`) names a command in the session's discovered native command
catalog (populated from the init `slash_commands` via the existing
`Event::Commands` path, `crates/monocode-core/src/provider/commands.rs:184-202`).
Raw commands skip every preamble injection (note, orchestrator, operator,
terminals, today) and are sent verbatim, following the OMP/Devin pattern.

Slash text that does not match the catalog keeps today's behavior: preambles
apply and it travels as an ordinary message. This keeps prose containing
`/`-prefixed words untouched.

Steers (mid-turn follow-ups, `crates/monocode-core/src/provider/follow_up.rs:30`)
already send message text without preambles and stay as they are.

**Catalog source and cold start.** The command catalog the condition matches
against lives UI-side, per conversation, only after that conversation's init
arrives — empty exactly for the first message of a new session, every Quick
Composer launch, and the pill's `×` after an app restart. The turn therefore
matches against the **union of** the conversation's in-memory catalog and a
**persisted last-seen catalog per provider kind**, written whenever any
session's init delivers `slash_commands`. From the second claude session of
an install onward the union is warm and all entry points work; the literal
first session of an install degrades to today's behavior (preamble + prose).
A `discover_commands` probe spawn was rejected: cost and complexity for one
edge case.

### 2. Composer Enter fixes

- Main composer: when the completion token is active and the choice list is
  empty, Enter submits the draft instead of only dismissing the picker.
  Non-empty list keeps the accept-without-submit behavior.
- Quick Composer: when the Commands picker has no match, Enter falls through
  to a normal launch (submit) instead of the silent `choose_command` return.

### 3. Goal state surface

- **Pill (placement A, decided):** in the row above the composer, same visual
  family as plan/orchestrator mode pills: `◎ 目标：<truncated condition>` with
  a trailing `×` that launches a raw `/goal clear` turn through the same
  passthrough. Clicking the pill body opens a popover with the full condition.
  No goal → no pill, no reserved space.
- **Transcript echoes:** the CLI does not replay command turns as user
  messages (observed on the wire: a set+clear session emits zero user rows),
  so the app inserts its own echo row when it launches a raw command turn —
  the same app-inserted-synthetic-row pattern `/compact` uses
  (`conversation/commands.rs:140-157`). Only recognized raw commands get the
  echo; unknown `/`-text stays an ordinary user row, so today's rendering of
  it is untouched. Lifecycle notices render as system rows; Stop-hook
  check-ins as thin dividers.
- **State model and resume:** monocode-core recognizes the CLI's synthetic
  goal lifecycle notices at the protocol layer (they arrive as assistant
  rows with `model: "<synthetic>"`) and emits a typed goal event; the gpui
  layer persists it as a typed transcript block (kind `goal`, with phase and
  condition). The pill derives from the last `goal` block in the session, so
  resume needs no extra storage and no re-parse of rendered text.

The structured `active_goal` event the CLI carries internally
(`{condition, iterations, set_at, tokens_at_start, last_reason?}` / null) is
**not available on this transport**: it is emitted only as an internal SDK
QueryEvent and under `CLAUDE_CODE_REMOTE` (binary evidence, plus a live
set+clear run under Monoptah's exact spawn flags producing zero such events
in 199 output rows). Recognition is therefore notice-based.

**Notice → pill transition table.** All notices are version-coupled to the
claude CLI (verified against 2.1.325; `Goal set:` and `No goal set` observed
on the wire, the rest attested in the CLI's message templates):

| Wire notice (synthetic assistant row) | Pill |
|---|---|
| `Goal set: <cond>` | active(`<cond>`) |
| `Goal cleared: <cond>` | none |
| `No goal set` | none |
| `Goal achieved` (attested, TUI-side; treat as clear if it appears) | none |
| `Goal cleared after an unrecoverable (…): …` | none + error note in echo |
| `Goal paused · …` | paused variant |
| `Goal could not be achieved` | paused variant |
| `Goal condition is limited to N characters` | unchanged |
| anything unrecognized | unchanged (ordinary row, no state change) |

The matcher lives in one place in monocode-core with this table and the
version note attached.

## Constraints

- Repo rules apply: `native-dev` profile aliases, tests in one pass filtered
  to touched modules, Linux-portable GPUI tests (no macOS-only assumptions).
- Version coupling: the CLI's goal notices and `slash_commands` list are not
  a contract (verified against 2.1.311 and 2.1.325; the binary moved from
  Homebrew cask to `~/.local/share/claude` between the two). The matcher and
  the persisted catalog both tolerate unknown shapes.
- The terminals preamble must keep flowing for every non-command turn; the
  carve-out must not strip it from ordinary prompts that merely contain a
  slash word.

## Acceptance

- Typing `/goal 修完并验证` in the main composer (any session except the
  install's literal first): command echo row appears, pill appears, next
  turns are goal-gated (CLI Stop hook visible in behavior).
- `×` on the pill or `/goal clear`: cleared row, pill disappears — including
  after an app restart with the goal restored from blocks.
- Quit and resume the session: pill reappears from saved blocks.
- `/model`-style advertised commands execute; unknown `/nosuchcmd text`
  behaves exactly as today (preamble + ordinary message, ordinary user row).
- Bare `/goal` typed in either composer submits (reaches the CLI) rather
  than being swallowed.
- A goal cleared by an unrecoverable error removes the pill (no stale
  active state).
- Store/probe/GPUI suites for touched modules pass; full `npm run native:check`
  before any release per repo policy.

## Left to the builder

- Exact event name and block schema for goal lifecycle (shape decided, names
  open).
- Pill row anchoring: the composer's pill toolbar sits below the input and
  the strip above it holds worktree/handoff notices
  (`conversation.rs:3151-3171`); "above the composer, pill family" is the
  decided placement, the exact slot follows the existing `tool_pill`
  conventions.
- Where the pill reads its condition-truncation length (match existing
  pill conventions).
- Whether the quick composer needs the provider command list at all after
  the fall-through fix (likely no; confirm no regression for the four mode
  commands).
- Test placement and naming per module conventions.
