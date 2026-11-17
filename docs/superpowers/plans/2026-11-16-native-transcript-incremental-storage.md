# Native ordinary-session incremental storage (batch 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (the user already chose subagent division for this effort; the batch-1 rulings in `.superpowers/sdd/2026-11-16-native-persistence-safety/progress.md` — one whole-request review, one test pass per task, no commits without authorization — bind this plan too). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ordinary Native local sessions stop rewriting their whole completed history on every save: a chunked backend in the same main SQLite database stores blocks sparsely (block-level and intra-block), cutting 40 MiB-history hot process writes by ≥95% versus the retained same-workload baseline.

**Architecture:** A new `native_chunks` storage mode selected by an explicit per-session mode/revision marker. Each block is a skeleton row (verbatim JSON with a rows placeholder) plus ordered row chunks; saves touch only dirty blocks and only dirty chunks. The existing blob path stays untouched for blob-mode sessions; `write_rows`/`write_rows_with_message` delegate by mode so no caller bypasses conflict checks or receipt transactionality. Readers, search, cloud/side-thread/today paths, conversion and downgrade export are completed before any existing session is converted.

**Tech Stack:** Rust in `crates/monocode-core` (rusqlite, serde_json), the batch-1 probe (`examples/transcript_save_probe.rs`) extended to schema v2, the batch-1 comparator extended to v2, GPUI tests only where UI order is pinned.

**Spec:** `docs/superpowers/specs/2026-11-16-native-transcript-write-amplification-design.md` (authoritative; sections 固定约束, 测量先行, 第二批：普通会话的增量存储, 数据转换及回退, 验收门槛). Batch-1 plan `docs/superpowers/plans/2026-11-16-native-persistence-safety.md` section "第二批：增量存储单独计划的固定输入" lists the seven fixed inputs this plan decomposes.

## Global Constraints

Copied from the spec (binding every task):

- Native only; do not modify `src/` or `src-tauri/` functionality.
- Build/verify with the `native-dev` profile (`--manifest-path crates/monocode-gpui/Cargo.toml --profile native-dev`); one test pass per task over the touched modules only; never plain `cargo test`/`clippy`.
- No write-number wins by weakening durability: no journal-mode changes, no lower `synchronous`, no fsync removal, no unlimited checkpoint deferral or memory backlog. Default auto-checkpoint stays.
- Keep the 650 ms routine save cadence and all existing force-save boundaries; queue dequeue, message writes and delivery receipts stay in one transaction in the same database.
- Uncommitted work only: this plan authorizes no commit, push, merge, release, or conversion of any production database. Experiments use synthetic databases in temp dirs only.
- Preserve identifiers: `monocode.*` keys, `monocode-host`, package/crate names, stored `INTERRUPT_MESSAGE`, and the blob schema for blob-mode sessions.
- Stats and probe output stay numeric enums/counts only; null is never zero; no user message bodies, tokens, or credentials in any output. macOS `ri_diskio_byteswritten` only under cfg(unix).
- Tests are Linux-portable (no macOS-only APIs, `secondary-` shortcuts); timing assertions use channel barriers, never sleep.
- Ordinary sessions are not marked Mono; no reuse of `mono_transcripts` markers, last-ten-turn origin rules, or cloud rejection logic (spec L99).
- One whole-request code review for the batch, plus the separate data-safety review for migration/delete/downgrade paths (spec L45) — Task 8 dispatches both.
- Every task that changes save behavior must be measurable by the batch-1 protocol: same fixture/workload/manifest, paired Before/After, honest "incomplete" marking (spec L47, L63-72).

## Review Focus

Failure modes the spec implies but no happy-path test exercises; each pinned by the named test in its owning task:

1. An older binary opens a database whose sessions use a newer storage mode → it must refuse to write (not treat as empty history, not write blob alongside). Test: `older_writer_refuses_newer_mode` (Task 3).
2. An in-place edit with identical timestamp and blob length must still invalidate search cache (fingerprint trap, spec L94). Test: `same_time_same_length_edit_invalidates_search` (Task 6).
3. Conversion interrupted between backup and mode switch (process kill, FULL, BUSY) → old format still valid and retryable, no half-converted session. Test: `interrupted_conversion_keeps_blob_valid` (Task 7).
4. Receipt-transaction failure must not dequeue the queue or lose/duplicate the message. Test: `failed_receipt_transaction_keeps_queue` (Task 4).
5. Two connections modify one session → the stale write is rejected and the other side's committed content survives. Test: `stale_write_rejected_other_side_preserved` (Task 4).

---

### Task 1: Probe schema v2 and comparator v2 (measurement tools only, no save-path change)

**Files:**
- Modify: `crates/monocode-core/examples/transcript_save_probe.rs`
- Modify: `scripts/compare-native-persistence.mjs`, `scripts/compare-native-persistence.test.mjs`
- Modify: `docs/native/persistence-profiling.md` (v2 contract section)

**Interfaces:**
- Consumes: batch-1 schema-contract v1 (`.superpowers/sdd/2026-11-16-native-persistence-safety/schema-contract.md`), the eight fixed scenarios and their manifests (unchanged bytes).
- Produces: `schema_version: 2` probe JSON. New fields (all null-never-zero): `backend` object per run (`{kind: "blob"|"native_chunks", mode_version: u32|null}`); per-sample `blocks_written: u64|null`, `chunks_written: u64|null`, `chunk_bytes_written: u64|null`; new stage `conversion` (only populated when the workload converts a session; timings and process-write deltas as the other stages); `summary.totals` gains the three new counters. Comparator `compareRuns` accepts v2↔v2 only, rejects v1↔v2 and v2↔v1 with explicit field-path reasons, and re-derives every new total from samples.

- [ ] **Step 1: Write the failing tests** — probe self-tests: v2 pins `schema_version: 2`, `backend.kind` present, new counters null on non-macOS exactly as v1 counters; comparator tests: v2 pair with mismatched `backend.mode_version` is rejected; v1-vs-v2 rejected with reason `schema_version`; totals re-derivation catches a lying summary.
- [ ] **Step 2: Run them to verify they fail** — `cargo test --manifest-path crates/monocode-gpui/Cargo.toml --profile native-dev -p monocode-core --example transcript_save_probe` and `node --test scripts/compare-native-persistence.test.mjs`; expected: new tests fail, existing 7+14 still pass.
- [ ] **Step 3: Implement** — extend the probe's sample/stage structs and JSON emission; keep every v1 field name and the manifest bytes identical (fixture/workload SHAs must not move); extend the comparator's shape/consistency checks per Interfaces.
- [ ] **Step 4: Run both suites once; all pass.** Document the v2 contract in the profiling doc's comparison section (one paragraph + field list, mirroring the v1 contract text).

### Task 2 (controller): freeze the batch-2 Before on the unchanged old path

**Files:**
- Create: `.superpowers/sdd/2026-11-16-native-transcript-incremental/results/batch-2/runs/...` per the batch-1 raw-artifact convention; `before-freeze-summary.md`; `experiment-cost.json` seeded.

- [ ] Run all eight scenarios once each with the v2 probe on the blob path (no behavior edits since Task 1 — verify via the packaged-diff isolator: zero non-probe/comparator/doc changes). All complete, integrity true, `backend.kind = "blob"`.
- [ ] Cross-check against the retained batch-1 baseline (v1 tool, same manifest): whole-run and hot-stage process writes must agree within run-to-run noise; record the deltas in the summary. This is the re-paired baseline the spec's L72 requires (tool changed → new pair), and the batch-1 runs remain batch-1's evidence.

### Task 3: Chunked storage layout and the mode/version write barrier

**Files:**
- Create: `crates/monocode-core/src/store/native_chunks/mod.rs`, `crates/monocode-core/src/store/native_chunks/layout.rs`
- Modify: `crates/monocode-core/src/store/shared.rs` (open-time barrier), `crates/monocode-core/src/store.rs` (table creation for new databases)
- Test: `crates/monocode-core/src/store/native_chunks/tests.rs`

**Interfaces:**
- Produces (later tasks consume):
  - `pub(crate) const NATIVE_CHUNKS_MODE_VERSION: u32 = 1;`
  - `pub(crate) enum StorageMode { Blob, NativeChunks { mode_version: u32 } }`
  - `pub(crate) fn session_storage_mode(conn: &rusqlite::Connection, session_id: &str) -> rusqlite::Result<StorageMode>;` (absent marker ⇒ `Blob`)
  - `pub(crate) fn ensure_native_chunks_tables(conn: &rusqlite::Connection) -> rusqlite::Result<()>;` — creates `native_sessions(session_id TEXT PRIMARY KEY, mode_version INTEGER NOT NULL, session_revision INTEGER NOT NULL)`, `native_blocks(session_id TEXT, block_seq INTEGER, block_id TEXT, skeleton_json TEXT NOT NULL, first_row INTEGER NOT NULL, row_count INTEGER NOT NULL, block_revision INTEGER NOT NULL, PRIMARY KEY(session_id, block_seq))`, `native_chunks(session_id TEXT, block_seq INTEGER, chunk_ord INTEGER, rows_json TEXT NOT NULL, byte_len INTEGER NOT NULL, PRIMARY KEY(session_id, block_seq, chunk_ord))` — `rows_json` is the comma-joined serialization of consecutive row elements; the skeleton stores the block's raw JSON with the rows-array contents replaced by the literal placeholder `"%NATIVE_ROWS%"`.
  - `pub(crate) fn block_raw_json(skeleton: &str, chunks: &[String]) -> Result<String, String>;` — text splice reconstruction (byte-stable for unchanged parts; unknown fields verbatim).
  - Open-time write barrier in the main opener: any session (or future whole-DB marker) whose `mode_version` exceeds `NATIVE_CHUNKS_MODE_VERSION` makes the whole connection refuse writes with a distinct error kind `UnsupportedStorageMode` (surfaced, not swallowed).

- [ ] **Step 1: Failing tests** — `round_trip_preserves_unknown_fields_and_hidden_blocks` (build a synthetic block set with unknown fields/hidden blocks via the existing rich adapter fixtures; write skeleton+chunks; reconstruct; compare via serde_json::Value equality and the probe's block fingerprint); `oversized_row_is_single_chunk`; `older_writer_refuses_newer_mode` (insert a fake `mode_version = NATIVE_CHUNKS_MODE_VERSION + 1` row; the barrier rejects writes with `UnsupportedStorageMode`); `existing_blob_sessions_untouched` (marker absent ⇒ Blob; blob read/write path byte-identical).
- [ ] **Step 2: Verify they fail** (module filter `native_chunks`), then **Step 3: implement** the DDL, marker read, reconstruction splice, and barrier hook in the opener. **Step 4: one pass** — `cargo fmt`, `cargo clippy … -p monocode-core --all-targets -- -D warnings`, `native_chunks` tests + full `store::` filter once.

### Task 4: Sparse block-level writer with conflict refusal and receipt-preserving delegation

**Files:**
- Create: `crates/monocode-core/src/store/native_chunks/writer.rs`
- Modify: `crates/monocode-core/src/store/shared.rs` (`write_rows`/`write_rows_with_message` dispatch by `StorageMode`; blob bodies unchanged), `crates/monocode-core/src/store/shared/blocks.rs` (dirty tracking feeds sparse plan), `crates/monocode-core/src/runtime/persistence.rs` (`PendingSave` expresses non-adjacent dirty ranges)
- Test: extend `native_chunks/tests.rs`, `store/shared/tests.rs`, `runtime/persistence.rs` safety tests

**Interfaces:**
- Consumes: Task 3 layout + `StorageMode`.
- Produces:
  - `pub(crate) struct DirtyBlock { pub block_seq: i64, pub first_row: i64, pub last_row: i64 }` (inclusive absolute rows)
  - `pub(crate) struct SparseSavePlan { pub session_revision_expected: i64, pub dirty: Vec<DirtyBlock> }`
  - `pub(crate) enum FlushConflict { StaleSession, StaleBlock { block_seq: i64 } }`
  - `pub(crate) fn flush_sparse(conn: &mut rusqlite::Connection, session_id: &str, plan: &SparseSavePlan, render: &dyn Fn(&DirtyBlock) -> Result<BlockUpdate, String>) -> Result<i64 /*new session_revision*/, FlushConflict-or-rusqlite::Error>;` — `BEGIN IMMEDIATE`, re-read `session_revision` and touched `block_revision`s, refuse on mismatch, bump revisions of touched blocks + session, `COMMIT`. `BlockUpdate { skeleton: String, rows: Vec<String> }`.
  - `PendingSave` gains `dirty: Vec<DirtyBlock>` alongside the existing window payload (blob path keeps using the window; the two never mix for one session).

- [ ] **Step 1: Failing tests** — `sparse_flush_touches_only_dirty_blocks` (probe-style process-write comparison on a small synthetic DB: dirty one early block, assert untouched blocks' rows_json byte-identical and `chunks_written` counts only dirty); `early_tool_plus_tail_token_one_commit` (non-adjacent dirty ranges flush in one transaction; middle blocks not re-read into the write set — assert via `history_bytes_read` sample staying bounded); `failed_receipt_transaction_keeps_queue` (inject commit failure via the batch-1 `fail_next_save_commit_for_test`; queue row, message row and receipt all absent-or-present together); `stale_write_rejected_other_side_preserved` (two connections; B commits; A's flush with stale expected revision → `FlushConflict::StaleSession`, B's rows intact); `zero_dirty_no_write` (no-op flush writes no transcript bytes — same-value acceptance, spec L123); `duplicate_or_missing_block_id_refused` (a sparse plan referencing an unknown block_seq, or two blocks claiming one block_id, is rejected with a distinct error, never silently dropped — spec fixed-input 4); `display_publishes_before_save_completes` (runtime persistence ordering test with a parked Store mutex, mirroring the batch-1 barrier style).
- [ ] **Step 2: verify fail; Step 3: implement** (dispatch in `write_rows*`, conflict refusal per spec L103 "重新加载或拒绝保存" — refuse and surface, do not auto-reload); **Step 4: one pass** — fmt, clippy, `native_chunks` + `store::` + `runtime::persistence` filters once.

### Task 5: Intra-block chunking for large growing blocks

**Files:**
- Create: `crates/monocode-core/src/store/native_chunks/chunking.rs`
- Modify: `crates/monocode-core/examples/transcript_save_probe.rs` (the large-tool and long-answer scenarios must exercise chunked appends once the session is native-mode; scenario bytes/manifest unchanged)
- Test: extend `native_chunks/tests.rs`

**Interfaces:**
- Consumes: Task 4 writer (`BlockUpdate.rows` per dirty block).
- Produces: `pub(crate) const CHUNK_TARGET_BYTES: usize = 65536;` and `pub(crate) fn chunk_rows(rows: &[String], existing: &[(i64, &str)]) -> Vec<ChunkOp>;` — pack consecutive serialized rows into chunks near `CHUNK_TARGET_BYTES`; row boundaries only; a single row larger than the target is its own chunk (`ChunkOp::{Rewrite(chunk_ord, rows_json), Append(rows_json), Truncate(keep)}`).

- [ ] **Step 1: Failing tests** — `append_rewrites_only_tail_chunk` (grow a 5 MiB-equivalent synthetic block by 1 KiB × 4; `chunk_bytes_written` per save bounded by ~target+1 KiB, not message size); `early_row_edit_rewrites_only_its_chunk`; `oversized_row_chunked_alone`; probe: large-tool/long-answer scenarios on native mode report bounded `chunks_written`/`chunk_bytes_written` with history size fixed (10 vs 40 MiB variant of append — logical hot writes must not grow; spec L125).
- [ ] **Step 2: verify fail; Step 3: implement** (chunk layout applied inside `flush_sparse`'s render path for native sessions); if the measured tail-chunk cost exceeds ~2× target on the probe scenarios, adjust `CHUNK_TARGET_BYTES` once and record the reason in the task report (spec L104: benchmarks decide layout). **Step 4: one pass** — fmt, clippy, `native_chunks` + probe example tests once.

### Task 6: Reader completeness and stable absolute row numbers

**Files:**
- Create: `crates/monocode-core/src/store/native_chunks/reader.rs`
- Modify: `crates/monocode-core/src/store/shared/window.rs` (mode dispatch; blob path untouched), `store/shared/index_search.rs` (revision enters the fingerprint), `store/today_sessions.rs` (backfill reads native mode), `store/side_threads.rs` (cleanup + snapshot full-history semantics), `store/shared/cloud_rows.rs` + `store/cloud_sessions/move.rs` (native sessions keep persistent row numbers; the existing indexed-history rejection stays for Mono indexed, not for native chunks), fork/edit/delete/restore entry points per the audit in `docs/native/persistence-profiling.md`
- Test: extend `native_chunks/tests.rs`, `store::` filters

**Interfaces:**
- Consumes: Tasks 3-5.
- Produces: `pub(crate) fn iter_full_history(conn, session_id) -> impl Iterator<Item = Result<RawBlock, String>>;` (reconstructed blocks in order) and `pub(crate) fn page_rows(conn, session_id, first_row, limit) -> Result<Vec<Row>, String>;` (UI pagination; never shifts the row origin). Row numbering: `native_blocks.first_row` assigns absolute rows at write time and never renumbers; `moved_at_row`, return-local gate and turn receipts keep their values.

- [ ] **Step 1: Failing tests** — `restart_equality_rows_and_raw_blocks` (probe integrity fingerprints green on native mode); `search_after_edit_same_time_same_length` (`same_time_same_length_edit_invalidates_search` per Review Focus 2); `today_backfill_reads_native`; `side_thread_cleanup_and_snapshot_full_history`; `cloud_move_preserves_row_numbers_and_gate`; `fork_edit_delete_restore_on_native`; `pagination_does_not_shift_origin` (page, then page again with different limit; same rows keep the same numbers).
- [ ] **Step 2: verify fail; Step 3: implement** reader + dispatch at each audited site; each site's blob behavior must remain covered by its existing tests (no deletions). **Step 4: one pass** — fmt, clippy, `native_chunks` + `store::` once. New sessions may now opt into native mode in the store layer (a `Store::create` flag or explicit session creation parameter — pick whichever matches existing session-creation call sites; conversion of existing sessions is still Task 7).

### Task 7: Per-session conversion, downgrade export, and recovery

**Files:**
- Create: `crates/monocode-core/src/store/native_chunks/convert.rs`
- Modify: `crates/monocode-core/src/store/shared.rs` (conversion entry point + write-enable gate)
- Test: extend `native_chunks/tests.rs`

**Interfaces:**
- Consumes: Tasks 3-6 (readers complete — the spec's precondition for enabling writes on existing sessions, L112).
- Produces:
  - `pub(crate) fn convert_session_to_native(conn: &mut Connection, session_id: &str) -> Result<ConversionReport, ConversionError>;` — one `BEGIN IMMEDIATE` transaction: consistency backup via the SQLite backup API into the app's backup location (never `cp` of a live WAL database), chunk build, marker insert, blob column left in place but the mode marker switches readers; commit. `ConversionReport { blocks, chunks, bytes_written }`.
  - `pub(crate) fn export_downgrade_to_blob(conn: &mut Connection, session_id: &str) -> Result<(), String>;` — rebuilds `blocks_json` from latest native content, verifies restart-equality fingerprints, switches the marker back in the same transaction (spec L116: verifiable downgrade that restores latest messages).
  - Interrupt/BUSY/FULL at any point ⇒ transaction rolls back, blob mode still valid, retry works.

- [ ] **Step 1: Failing tests** — `interrupted_conversion_keeps_blob_valid` (fail via `fail_next_save_commit_for_test`-style injection at the chunk-build and marker stages); `conversion_is_atomic_and_retryable`; `downgrade_export_restores_latest_and_verifies`; `old_blob_writer_cannot_silently_dual_write` (a blob-path `write_rows` against a native-mode session is refused with `UnsupportedStorageMode`, not a silent parallel write — spec L116).
- [ ] **Step 2: verify fail; Step 3: implement**; **Step 4: one pass** — fmt, clippy, `native_chunks` + `store::` once. This task's diff is the input to the separate data-safety review (Global Constraints).

### Task 8 (controller): After collection, acceptance gates, reviews, delivery

- [ ] Collect After (v2 probe, same manifest) for all eight scenarios on native mode; `conversion` stage numbers reported separately from hot saves (spec L124); comparator v2 pairs all comparable; fill `docs/native/persistence-profiling.md` batch-2 table (95% gate, 10→40 growth, large-message bound, conversion cost, experiment cost).
- [ ] p95 supplemental measurement: ≥100 labeled small-save samples in a controlled run, reported as p95/max with sample count (spec L69, L127); CI gets no timing assertions.
- [ ] Dispatch the whole-request review AND the separate migration/delete/downgrade data-safety review (spec L45); one fix round + one scoped re-review if needed (batch-1 precedent).
- [ ] Update the handoff ledger; report honestly which acceptance lines passed, which stay 待验收, and that live-UI verification remains pending.
