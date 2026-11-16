# Monoptah Native `/goal` Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Claude Code's `/goal` (and every CLI-advertised slash command) work in Monoptah Native, with a goal-status pill, command echo rows, and state that survives resume.

**Architecture:** Three layers: (1) monocode-core learns to recognize goal notices on the claude stream (`Event::Goal`) and gets a pure `is_advertised_command` predicate; (2) the gpui runtime persists the per-provider command catalog on every init, sends advertised commands verbatim (no preambles), and inserts an echo row; (3) the conversation renders goal lifecycle rows and a pill derived by scanning transcript rows — no separate state to sync, resume free.

**Tech Stack:** Rust (monocode-core, monocode-gpui), GPUI, SQLite store (`setting`/`save_setting`).

**Spec:** `docs/superpowers/specs/2026-11-16-native-goal-support-design.md` — read it first; the plan argues from it.

## Global Constraints

- Build/test only with the `native-dev` profile: from `crates/monocode-gpui` use `cargo ntest`, `cargo nclippy`, `cargo fmt`. Never plain `cargo test`.
- One test pass per task, filtered to touched modules; no full `npm run native:check` until the final task.
- GPUI tests must be Linux-portable: no macOS-only paths, `secondary-...` keystrokes only.
- Commit style: sentence-case subject, no `feat:` prefix (match `git log --oneline -5`).
- Version-coupled CLI strings live in exactly one place (`provider/goal.rs`) with the design's transition table; do not scatter prefixes.
- The terminals/operator/today preambles must still apply to every non-command turn.
- Never rename `monocode.*` identifiers or weaken an assertion to pass a test.

## Review Focus

(Tests pinned inside the owning tasks, per line.)

1. **Unknown slash text keeps today's behavior** — `/nosuchcmd hello` gets preambles, an ordinary user row, no echo. (Task 3 test)
2. **Cold start with no persisted catalog** — missing setting parses as empty; turn degrades to prose, no panic. (Task 3 test)
3. **Prose containing a mid-sentence slash word is never raw** — predicate anchors at the trimmed start only. (Task 2 test)
4. **Error-cleared goal removes the pill** — transition table maps `cleared after an unrecoverable error` to no-goal; no stale active state. (Tasks 1, 7 tests)
5. **Resume derives the pill from saved rows** — last `goal` row wins; a trailing cleared/achieved row yields no pill. (Task 7 test)
6. **Recognized command still gets the echo when the in-memory catalog is cold but the persisted one is warm** (Quick Composer first message). (Task 3 test)

---

### Task 1: Goal notice parsing (monocode-core)

**Files:**
- Create: `crates/monocode-core/src/provider/goal.rs`
- Modify: `crates/monocode-core/src/transcript.rs` (add `GoalNotice` next to `Interjection` at `transcript.rs:9`)
- Modify: `crates/monocode-core/src/provider.rs` (module declaration + `Event::Goal` variant next to `Event::Interjection`, `provider.rs:103`)
- Test: `crates/monocode-core/src/provider/goal.rs` (inline `mod tests`)

**Interfaces:**
- Produces: in `transcript.rs` (next to `Interjection`): `pub struct GoalNotice { pub phase: GoalPhase, pub condition: Option<String>, pub notice: String }` and `pub enum GoalPhase { Set, Checkin, Paused, Cleared, Achieved, Error }` (both `Clone, Debug, PartialEq, Eq, Serialize, Deserialize`; `GoalPhase` serde-renamed to lowercase `"set"` … `"error"`). In `provider/goal.rs`: `pub fn goal_notice(text: &str) -> Option<GoalNotice>` implementing the spec's transition table. `notice` is the full input text; `condition` is `Some` only for `Set`/`Cleared`.
- Produces: `Event::Goal(GoalNotice)` on `provider::Event`.
- Consumes: nothing new.

- [ ] **Step 1: Write the failing tests** (inline `mod tests` in `goal.rs`)

```rust
#[test]
fn parses_every_notice_in_the_transition_table() {
    let set = goal_notice("Goal set: reply with ok").unwrap();
    assert_eq!(set.phase, GoalPhase::Set);
    assert_eq!(set.condition.as_deref(), Some("reply with ok"));
    assert_eq!(goal_notice("No goal set").map(|g| g.phase), Some(GoalPhase::Cleared));
    assert_eq!(goal_notice("Goal cleared: reply with ok").map(|g| g.phase), Some(GoalPhase::Cleared));
    assert_eq!(goal_notice("Goal still active · 2 · retrying in 4s").map(|g| g.phase), Some(GoalPhase::Checkin));
    assert_eq!(goal_notice("Goal paused · the last turn could not finish · x").map(|g| g.phase), Some(GoalPhase::Paused));
    assert_eq!(goal_notice("Goal could not be achieved").map(|g| g.phase), Some(GoalPhase::Paused));
    let err = goal_notice("Goal cleared after an unrecoverable error (2): boom. Run /goal again to continue.").unwrap();
    assert_eq!(err.phase, GoalPhase::Error);
}
#[test]
fn unknown_text_is_not_a_goal_notice() {
    assert!(goal_notice("Here is your summary").is_none());
    assert!(goal_notice("").is_none());
    assert!(goal_notice("Goal condition is limited to 500 characters").is_none()); // info only, no state change
}
```

- [ ] **Step 2: Run to verify failure** — `cd crates/monocode-gpui && cargo ntest -p monocode-core goal::` — Expected: compile error (module missing).
- [ ] **Step 3: Implement** — `GoalNotice`/`GoalPhase` in `transcript.rs` (serde derives mirroring `Interjection`), `goal_notice` in `provider/goal.rs` as an ordered prefix/contains match over the table (a `Goal set: ` prefix extracts the condition as everything after the prefix, trimmed; other rows match their attested prefixes from the spec's table; `Goal achieved` → `Achieved`). Add `mod goal;` + `Event::Goal` variant.
- [ ] **Step 4: Run to verify pass** — `cargo ntest -p monocode-core goal::` — Expected: PASS.
- [ ] **Step 5: Commit** — `git add crates/monocode-core/src && git commit -m "Parse claude goal lifecycle notices in monocode-core"`

### Task 2: Advertised-command predicate (monocode-core)

**Files:**
- Modify: `crates/monocode-core/src/provider/commands.rs`
- Test: inline `mod tests` additions in `commands.rs`

**Interfaces:**
- Produces: `pub fn is_advertised_command(text: &str, names: &std::collections::BTreeSet<&str>) -> bool` — true iff the trimmed text starts with `/` and the first whitespace-delimited word after it (stripped of one trailing `:` disambiguation segment, i.e. `claude:goal` → `goal`) is contained in `names`.
- Consumes: nothing from Task 1.

- [ ] **Step 1: Write failing tests** in the existing `mod tests` (`commands.rs:205`)

```rust
#[test]
fn advertised_commands_match_at_the_start_only() {
    let names = BTreeSet::from(["goal", "model"]);
    assert!(is_advertised_command("/goal fix it", &names));
    assert!(is_advertised_command("  /model", &names));
    assert!(is_advertised_command("/claude:goal x", &names));
    assert!(!is_advertised_command("please run /goal now", &names));
    assert!(!is_advertised_command("/nosuchcmd hi", &names));
    assert!(!is_advertised_command("/", &names));
    assert!(!is_advertised_command("plain text", &names));
}
```

- [ ] **Step 2: Run** — `cargo ntest -p monocode-core advertised_commands` — Expected: FAIL (undefined).
- [ ] **Step 3: Implement** `is_advertised_command` in `commands.rs` near `native_prompt` (`commands.rs:30`).
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -am "Add an advertised-command predicate for provider slash passthrough"`

### Task 3: Runtime passthrough, catalog persistence, echo row (monocode-gpui)

**Files:**
- Modify: `crates/monocode-gpui/src/runtime.rs:483-493` (raw check), `:742-747` (Commands event), turn-start row appends near `:912`
- Test: `crates/monocode-gpui/src/runtime.rs` inline tests + `crates/monocode-core/src/provider.rs` framing test

**Interfaces:**
- Consumes: `is_advertised_command` (Task 2), `GoalNotice` (Task 1, only if framing lands here — see step 3), `store.setting`/`store.save_setting` (`crates/monocode-core/src/store.rs:260,275`).
- Produces: persisted setting key `provider-commands:{kind.id()}` whose value is a JSON array of command names (written on every `Event::Commands` for Claude); claude turns whose prompt passes `is_advertised_command` against the persisted names are sent verbatim with all preambles skipped; each such turn inserts an interjection echo row before launch: role `interjection`, body `Interjection { custom_type: "command", severity: None, text: <the full prompt line> }`.

- [ ] **Step 1: Write the failing core framing test** (in `provider.rs` tests, mirroring the frame tests at `provider.rs:2236`): a claude `assistant` wire row with `"model": "<synthetic>"` and text `"Goal set: reply with ok"` yields `Event::Goal` and **no** `Event::Text`; the same content with a real model yields `Event::Text` and no `Event::Goal`. Name: `synthetic_goal_rows_become_goal_events`.
- [ ] **Step 2: Run** — `cargo ntest -p monocode-core synthetic_goal_rows` — Expected: FAIL.
- [ ] **Step 3: Implement the framing intercept** in the claude `"assistant" =>` branch (`provider.rs:850-883`): after collecting `text`, if `v["message"]["model"] == "<synthetic>"`, run `goal_notice(&text)`; on `Some`, push `Event::Goal` and skip `self.text(...)`. Also implement here: in the runtime `Event::Commands` arm (`runtime.rs:746`) persist the names for Claude via `locked(&store)?.save_setting(...)`; at the raw check (`runtime.rs:489`) add the Claude arm reading the persisted names (`setting` → parse JSON array → `BTreeSet`, missing/corrupt → empty set, no error) and set `raw_native_command` from `is_advertised_command`; where raw claude command turns start, append the echo interjection row using the `transcript.begin("interjection")` + serialized-append pattern at `runtime.rs:912-924`.
- [ ] **Step 4: Write the runtime tests** — cold start: a claude turn with prompt `/goal x` and **no** persisted setting still prepends the terminals preamble and produces no echo row (assert on the built prompt + row roles via the scripted-run pattern used by `conversation/cloud_start_tests.rs`); warm catalog: with `provider-commands:claude` saved, the same prompt is sent verbatim (no preamble) and an `interjection` row with `custom_type == "command"` exists. If the scripted-run harness cannot inspect the outgoing prompt directly, assert through the persisted prompt receipt the runtime already stores, and say so in the task notes.
- [ ] **Step 5: Run both** — `cargo ntest -p monocode-core synthetic_goal_rows` and `cargo ntest runtime::` — Expected: PASS.
- [ ] **Step 6: Commit** — `git commit -am "Send advertised claude slash commands verbatim with a persisted catalog and echo row"`

### Task 4: Main composer Enter submits on no match (monocode-gpui)

**Files:**
- Modify: `crates/monocode-gpui/src/composer_completion.rs:1456-1465`
- Test: `composer_completion.rs` tests near `:1800`

**Interfaces:**
- Produces: behavior change only — Submit with an active `/` token and an empty choice list dismisses the picker **and lets the action propagate** (remove `cx.stop_propagation()` from the empty-choices branch only; the non-empty branch keeps accept-without-submit).

- [ ] **Step 1: Write the failing test** next to `slash_accept_keeps_input_focus_and_does_not_submit` (`composer_completion.rs:1800`): type `/goal`, ensure the choice list is empty (no native commands loaded), press Enter → the input's `Submitted` reaches the outer subscriber (reuse that test's `submitted` counter pattern). Name: `slash_enter_with_no_match_submits`.
- [ ] **Step 2: Run** — `cargo ntest slash_enter_with_no_match` — Expected: FAIL (counter stays 0).
- [ ] **Step 3: Implement** the branch change.
- [ ] **Step 4: Run** — Expected: PASS, and `slash_accept_keeps_input_focus_and_does_not_submit` still PASS.
- [ ] **Step 5: Commit** — `git commit -am "Submit the composer draft when slash completion has no match"`

### Task 5: Quick Composer falls through on no match (monocode-gpui)

**Files:**
- Modify: `crates/monocode-gpui/src/quick_composer.rs:439-443` (`launch`), possibly `:630-637` (`command_options`)
- Test: `quick_composer.rs` inline tests

**Interfaces:**
- Produces: behavior change only — Enter with the Commands picker open and `command_options` empty closes the picker and continues into the normal submit path instead of the silent `choose_command` return.

- [ ] **Step 1: Write the failing test**: input `/goal` (no whitespace → Commands picker open, no match in the static four), call `launch` → a `Launch` event is emitted with `prompt == "/goal"`. Name: `unmatched_command_submits_instead_of_silent_return`. (Match the existing quick-composer test setup for emitting/capturing `Launch`.)
- [ ] **Step 2: Run** — `cargo ntest unmatched_command_submits` — Expected: FAIL (no event).
- [ ] **Step 3: Implement** — in `launch`, route to `choose_command` only when options are non-empty; otherwise `close_picker_state` and fall through.
- [ ] **Step 4: Run** — Expected: PASS, and the four mode commands still route through `choose_command` (extend an existing mode-command test if one exists).
- [ ] **Step 5: Commit** — `git commit -am "Fall through to submit when the quick composer command list has no match"`

### Task 6: Goal transcript rows and rendering (monocode-gpui)

**Files:**
- Modify: `crates/monocode-gpui/src/runtime.rs` (`Event::Goal` arm next to `Interjection` at `:912`: `transcript.begin("goal")` + serialized `GoalNotice` append)
- Modify: `crates/monocode-gpui/src/transcript.rs` (`Body::Goal(Arc<GoalNotice>)` next to `Body::Interjection` at `:171`; row-kind mapping)
- Modify: `crates/monocode-gpui/src/mobile/transcript.rs:322` (one-line `"goal" =>` deserialization so mobile renders `notice`, not raw JSON)
- Test: `transcript.rs` inline tests

**Interfaces:**
- Consumes: `Event::Goal(GoalNotice)` (Task 1/3).
- Produces: transcript rows with role `goal`; `Body::Goal` renders `Set`/`Cleared`/`Achieved`/`Error` as a system-style row (reuse the interjection row styling with a `target` icon) and `Checkin` as a thin divider (reuse the `MonoDivider` styling approach); `Paused` as a subdued system row.

- [ ] **Step 1: Write the failing test**: feeding a `goal`-role row with serialized `GoalNotice { phase: Set, .. }` through the transcript body mapping yields `Body::Goal`, and a `Checkin` row maps to the divider variant. Name: `goal_rows_map_to_typed_bodies`.
- [ ] **Step 2: Run** — `cargo ntest goal_rows_map` — Expected: FAIL.
- [ ] **Step 3: Implement** the three file changes.
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -am "Render goal lifecycle rows and checkin dividers in the transcript"`

### Task 7: Goal pill, clear action, resume derivation (monocode-gpui)

**Files:**
- Modify: `crates/monocode-gpui/src/conversation.rs` (`goal_state` helper; pill in the notices strip above the composer at `:3145-3175`; `×` handler)
- Test: `conversation.rs` inline tests (follow `submission_tests.rs` setup patterns)

**Interfaces:**
- Consumes: `goal`-role rows (Task 6), `tool_pill(id, tooltip)` (`conversation/composer_features.rs`), the app `Popover` (`popover.rs`), `launch_prompt` (`conversation.rs:1299`).
- Produces: `fn goal_state(&self, cx: &App) -> Option<(GoalPhase, String)>` — reverse-scans `source_rows()` for the last `goal` row, deserializes `GoalNotice`, returns `Some` for `Set`/`Checkin` (active) and `Paused` (paused), `None` for `Cleared`/`Achieved`/`Error`/absent. Pill: `tool_pill("goal-state", <full condition>)` with `◎ 目标：<condition truncated>` text and a `×` child that calls `self.launch_prompt("/goal clear".into(), cx)`; clicking the pill body opens a popover with the full condition; rendered in the strip alongside `remote_notice`/`handoff` (no dedicated row when `goal_state` is `None`).

- [ ] **Step 1: Write the failing tests** — (a) `goal_state_reads_the_last_row`: rows `[Set("fix it"), Checkin]` → active with `"fix it"`; `[Set, Cleared]` → `None`; `[Set, Error]` → `None`; empty → `None`. (b) `goal_pill_clear_launches_the_raw_command`: conversation with an active goal, click `×` → `launch_prompt` receives `"/goal clear"` (capture via the scripted-run/test double used by existing conversation tests).
- [ ] **Step 2: Run** — `cargo ntest goal_state_reads` and `cargo ntest goal_pill_clear` — Expected: FAIL.
- [ ] **Step 3: Implement** helper + pill + handlers.
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -am "Show the active goal above the composer with one-click clear"`

### Task 8: Verification pass and spec status

**Files:**
- Modify: `docs/superpowers/specs/2026-11-16-native-goal-support-design.md` (Status → implemented)
- No product code.

- [ ] **Step 1: One full local pass** — from `crates/monocode-gpui`: `cargo fmt`, `cargo nclippy`, then the touched-module tests in one `cargo ntest` run (`goal:: advertised_commands synthetic_goal_rows runtime:: slash_enter unmatched_command goal_rows goal_state goal_pill slash_accept`).
- [ ] **Step 2: Manual acceptance in a dev build** — `npm run tauri:dev` is the Tauri app; for Native use a clean worktree dev run per CLAUDE.md, then walk the spec's Acceptance list (`/goal 修完并验证` → echo row + pill; `×` → cleared; quit/resume → pill back; `/nosuchcmd text` unchanged; bare `/goal` in both composers submits). Record results in the task notes; anything failing goes back to the owning task.
- [ ] **Step 3: Commit** — `git commit -am "Mark the goal support design implemented"` — and report the acceptance table to the controller.
