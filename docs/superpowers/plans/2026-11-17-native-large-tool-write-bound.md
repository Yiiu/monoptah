# Native Large-Tool Write Bound Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Continue inline in this session. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop rewriting giant tool display/metadata strings in every skeleton and report all committed native transcript JSON bytes.

**Architecture:** Reuse mode-1 payload segmentation at arbitrary oversized string locations; preserve exact content and both legacy/new canonical export validation. Add a separate skeleton counter, propagated through a schema-v3 probe/comparator without changing existing chunk-counter meaning.

**Tech Stack:** Rust/serde_json/rusqlite in native core; existing Node comparator and synthetic probe.

**Spec:** `docs/superpowers/specs/2026-11-17-native-large-tool-write-bound-design.md`, extending the 2026-11-16 native transcript design.

## Global Constraints

- Native only; preserve prior safety edits and all stored identifiers.
- Use Rust 1.97.1 and `native-dev`; retain the current dedicated branch/build cache.
- Batch edits and related validation; fix and rerun only failures. One final code review for this request.
- Synthetic temporary databases only; no providers, production data or credential output.
- Keep 650 ms cadence, transaction/receipt boundaries and existing durability/checkpoint settings.
- No commit, push, merge, release, production conversion or old-session conversion UI.
- Retain every experiment attempt and unknown counters. Windows functional/logical results do not establish macOS process-I/O or UI acceptance.

## Review Focus

- Giant Unicode strings inside nested previews, arrays and unknown metadata must round-trip exactly (Task 2).
- Previously saved mode-1 blocks must downgrade before any normalization save, and save/reload correctly afterwards (Task 2).
- Reserved marker literals must remain refused before stripping; no placeholder can inject source text (Task 2).
- A failed commit must expose zero committed-write counters and keep pending content/retry behavior (Task 1).
- Native-null/blob-reason and blob-numeric counter reports must be rejected, not silently treated as comparable (Task 1).

---

### Task 1: Honest native JSON-write accounting and paired measurement tools

**Files:** `store/native_chunks/writer.rs`, `store/save_stats.rs`, `store/shared.rs`, `examples/transcript_save_probe.rs` under `crates/monocode-core`; `scripts/compare-native-persistence.mjs` and its test file; profiling docs.

**Interfaces:** Add `skeleton_bytes_written: u64` to `SparseFlush` and `SaveSample`; count actual skeleton parameters submitted by changed-block upserts and expose them only on successful commit. Keep `chunk_bytes_written` unchanged. Probe v3 exposes nullable skeleton bytes/missing reason and sum/max aggregates; blob is null/`backend:blob`, native is numeric. Derived native transcript JSON bytes = skeleton + chunk bytes. This excludes scalar/index/WAL/filesystem bytes.

- [x] Add regression assertions for the new field in exact-key/summary/native/blob probe tests, rollback stats, schema-v2 rejection, summary tampering, and counter/backend contradictions.
- [x] Run the related RED batch once; retain failures and identify their causes.
- [x] Add skeleton accounting at the upsert, propagate it, clear committed counters on failed samples, and update the probe/comparator to required schema v3 without changing fixtures or chunk semantics.
- [x] Run measurement-tool tests only; once green, build and retain one instrumented pre-layout large-tool run/executable. This is the Before baseline for logical results; preserve null OS counters on Windows.

### Task 2: Segment giant string members and preserve legacy export

**Files:** `store/native_chunks/layout.rs`, `store/native_chunks/convert.rs`, `store/native_chunks/tests.rs` under `crates/monocode-core`.

**Interfaces:** Keep `store_block_form(value, raw, rows) -> Result<StoredForm, String>` and the existing generic reader. Add an internal legacy stored-form builder for exact old-layout export verification. Oversized strings use the existing payload marker in serialization traversal order, except root block ID; row-reconstructible plain text retains its text sentinel.

- [x] Add tests for large tool text/title/preview strings, nested unknown/Unicode values, marker refusal, and exact reconstructed values.
- [x] Extend the existing 5 MiB incremental test to bound skeleton + chunk bytes, including no-change zero and preservation of unrelated blocks.
- [x] Seed actual legacy forms to verify both immediate downgrade and first save/reopen, retaining source values and receipts.
- [x] Include these tests in the RED batch in Task 1; then implement recursive oversized-string stripping, avoiding an extra joined-text allocation in the inverse-text check.
- [x] Keep downgrade's strict stored-form/row/content checks; accept either exact current or exact legacy canonical form. Do not bump mode version because readers already restore arbitrary-position payload markers.

### Task 3: One validation/review pass and an honest result record

**Files:** spec/plan and `docs/native/persistence-profiling.md`; ignored/temp experiment outputs.

- [x] Run formatting, core clippy `--all-targets -- -D warnings`, core tests filtered by `store::`, probe example tests, and comparator Node tests in one pass. Fix/rerun only failures.
- [x] Run the same large-tool workload After using schema v3. Verify manifest, SQLite config, full content/restart fingerprints and all event counts against Before. Keep raw exit codes and all attempt costs.
- [x] Report logical skeleton/chunk differences separately from unavailable OS-write improvements. Record remaining encoding/read/UI work; never infer the 95% disk target from logical counters.
- [x] Hand one reviewer the original request/spec, changed-file checklist and test evidence; include legacy-export safety in that review. Address verified blockers in one fix pass.
- [x] Update the ledger and final result with actual counts, paired evidence and outstanding Mac/Native Dev acceptance. Leave all work uncommitted.

## Evidence ledger

- RED task `bwd1v7v5w` (exit 101): compilation/clippy passed; native tests 44 passed, 4 runtime failures reproduced giant skeleton and old-layout save bounds. Probe 19/19 and comparator 21/21 passed; instrumented old-layout executable built. This was not a compile/helper failure.
- Before controller `b5w887rqj` (exit 0): all five large-tool events and integrity checks passed. Raw probe exit 2 solely for `process_io_unavailable`.
- Combined validation `btwyb7n77` (exit 0): native formatting, core all-targets clippy, 466 store-related tests, 19 probe tests, 21 comparator tests and runnable probe build passed. Existing dependency linker messages and one probe-test `unused_mut` warning remain.
- After controller `bvipjz8my` (exit 0): all five events/content/restart checks passed and matched Before's manifest/configuration/toolchain/lock/fingerprints. Raw probe exit 2 solely for unavailable OS counters.
- Hot transcript JSON: 41,987,352 → 10,530,216 B (74.92% lower). Last three appends: 31,496,658 → 39,486 B (99.87% lower); first-caption creation remains in the full hot total. Changed skeletons: about 10.49 MB → 515 B. Final same-value writes: zero on both sides.
- Attempted encoding remains 78,688,428 B on both sides; full active-block reconstruction/projection/encoding and UI latency remain separate work.
- Strict process-I/O comparator returned exit 1 / `comparable:false`, preserving all missing-counter reasons. Two measurement attempts have unknown costs; total stays null. No disk-write or 95% acceptance claim.
- Artifacts: `C:/Users/a1103/AppData/Local/Temp/monoptah-large-tool-write-bound-20261117-bwd1v7v5w/`. Raw reports, manifests, invocations, immutable binary copies, logical comparison, strict rejection and cost ledgers retained. Full hashes are in the profiling document and raw reports.
- One independent final review completed all five implementation/safety checklist points and reported no concrete correctness defects or blocking findings. It independently inspected the retained logical pair, read code without duplicating broad tests, and made no edits. No second whole-request review was requested.
- Pending external acceptance: matched macOS process-I/O measurements and Native Dev from a clean checkout matching origin. Production conversion/UI wiring and all publishing remain outside this task.
