# Native persistence profiling

The core probe measures synthetic transcript saves into a new temporary blob-schema database. It accepts exactly `--scenario ID`, never a database path, and never calls a provider. Use no production sessions, credentials or app data. Its stdout is one `schema_version: 2` JSON object; exit 0 means complete, exit 2 means incomplete or skipped.

## Not yet migrated: synchronous UI Store calls

Task 3's first batch moves only the Inbox's `ViewState` save onto the fixed-key background saver (`ui_store_saves.rs`, key `native-inbox-state`). Everything below still takes the Store on its original thread and is deliberately out of scope for the setting saver: these calls need transactional business receipts, result-dependent UI updates, or dedicated lifecycle designs of their own. Line numbers are from the controller's read-only audit at baseline d2675a25 and may move.

| Location | Context and result dependency | Batch scope |
| --- | --- | --- |
| notes.rs:296 | persist runs from UI debounce/flush, upsert result updates note and clears dirty; selecting a note waits for successful flush | Still synchronous; needs dedicated result/dirty/lifecycle design, not a setting saver |
| notes.rs:386,407 | create/delete update current selection and note list synchronously | Still synchronous; result-dependent operations, delete requires own safety review |
| notes.rs:570,826 | UI callback upsert / note lookup | Still outside current Inbox scope |
| conversation.rs:2515 | provider change + optional provider_id restoration happen before applying model pick; failure stops UI state change | Still synchronous; transaction/result semantics differ from latest-value settings |
| conversation/mono.rs:341 | mono_patch_card writes row before publishing transcript patch | Still synchronous; persistence acknowledgement must follow successful save |
| conversation/mono.rs:465 | mono_post writes report rows before emitting SessionChanged | Still synchronous; do not move to fixed-key setting saver |
| shell.rs:1066 | new_with_metadata obtains data_dir during UI construction | Still synchronous startup/view construction |
| shell.rs:1608 | Telegram setup data-dir lookup through Store | Still synchronous; outside Inbox scope |
| shell.rs:4673 | save_state already uses background executor and checks generation after acquiring Store | Already off UI for this operation, not a remaining synchronous save |
| shell.rs:1134 | application quit workspace-save hook uses background executor | Already off UI, separate best-effort lifecycle |
| shell.rs:2056,2105–2112,4174,5222,6676 | surrounding async/background operations | Do not classify a lock hit alone as UI blocking |

Other conversation constructor/history and Mono/cloud/remote paths were not fully audited by that pass. Do not claim all UI locking is fixed. The Inbox constructor still loads state and `data_dir` synchronously (inbox.rs `new_inner`), and the Inbox refresh still loads sessions on the background executor and may wait for the Store even after the UI save fix. Task 4's coverage list starts from this table.

This batch retains the transcript blob format. The write-reduction target of 95% belongs to the second batch and is pending acceptance. The first batch may improve UI lock behavior, but the core probe has no UI or GPUI lock measurements. Historical **549.76 GB** diagnostic data is not this Before baseline and must not appear as a measured Before value.

## Freeze Before before Task 3

The controller owns measurement and validation. Finish measurement-tool formatting, lint and related tests first. Then freeze Before using the instrumented old save path, **strictly before Task 3 Inbox edits**. Task 2 supplies the comparator and these instructions; it has not collected or finalized measurements.

Use the native workspace's `native-dev` profile and its `Cargo.lock`. Build identity is embedded at compilation: record `build.source_sha`, `diff_sha256`, `source_tree_sha256`, `toolchain`, `cargo_lock_sha256` and `profile` from each raw result. The diff hash is the producer's length-delimited source/diff identity, not simply a hash of textual `git diff`. Later working-tree changes do not change an existing binary's identity. Record the exact binary/invocation used; do not label a stale build as the current source.

Run each scenario with the same fixture and declared workload on both sides. Each process has a **536870912 byte (512 MiB)** absolute process-write budget and a **120000 ms** wall limit. One save, checkpoint or close may overshoot; retain that result and mark it incomplete. The producer gates saves inside the shared Store mutex. Do not increase budgets, weaken SQLite durability or remove/coalesce events to make a run finish. Do not repeat the old large scenarios for statistical sampling. An incomplete large Before remains incomplete, with no improvement claim.

The eight IDs are `append-1m`, `append-10m`, `append-40m`, `same-value`, `early-tool`, `large-tool`, `long-answer` and `parallel`. Most have four hot saves per session at 0/650/1300/1950 ms and a final save at 2600 ms. `early-tool` has one hot event and one final event; `parallel` has four sessions in tick/session manifest order. Actual starts/completions can be late or reordered in parallel; preserve all declared events and raw timing.

If the measurement tool fails, repair only that tool, retain the failed raw attempt and its cost, and re-establish the affected baseline before business edits. If tools, fixture, workload, configuration, lock or toolchain change, establish a new matching Before/After pair. Do not overwrite an earlier attempt. A representation change needs an explicit schema-contract revision: version 1 compares content fingerprints within the unchanged blob schema.

## Raw artifact convention

Keep this batch under the following directory (local experiment artifacts, not source fixtures). `<batch>` identifies one frozen measurement setup; `<run-id>` is a unique attempt identifier such as a UTC timestamp.

```text
.superpowers/sdd/2026-11-16-native-persistence-safety/results/<batch>/
  runs/<scenario>/before-<run-id>/probe.json
  runs/<scenario>/before-<run-id>/manifest.json
  runs/<scenario>/before-<run-id>/invocation.json
  runs/<scenario>/before-<run-id>/stderr.log
  runs/<scenario>/after-<run-id>/...             # same files
  comparisons/<scenario>-<before-id>-<after-id>.json
  experiment-cost.json
```

`probe.json` is untouched stdout, including failed/skipped/partial attempts. Copy its exact `manifest` into `manifest.json` (keep `null` if setup failed). `invocation.json` records the binary path, scenario/arguments, start/end time, exit status and the raw file path; the complete build identity stays in `probe.json`. Keep stderr separate from JSON; review it for secrets before sharing. Never put credentials or production content in these artifacts. If a process dies without valid JSON, retain its raw output and record the missing final process counter as unknown.

`experiment-cost.json` is the controller's ledger for **every** attempt on both sides, including discarded runs, tool failures, skips and budget overshoots. Record each raw path and `process_lifetime_written_bytes`, then sum the known final absolute counters. If any counter is null/unavailable, report the known subtotal and unknown-run count; the full experiment cost remains unknown. Do not replace unknown counters with zero or hide setup/failed work by restarting a process. A comparator report's pair cost covers only its two inputs and explicitly excludes other attempts.

Once both raw files exist, create the comparison directory and run, substituting the retained paths:

```sh
node scripts/compare-native-persistence.mjs \
  --before <before-run-directory>/probe.json \
  --after <after-run-directory>/probe.json \
  --out <batch-directory>/comparisons/<scenario>-<before-id>-<after-id>.json
```

Read the command's exit status directly. Exit **0** means complete and comparable, independent of whether the 95% target is met. Exit **1** means invalid input, incomplete measurements or inconsistent data/configuration. For parsed runs the output includes rejection reasons; missing/unreadable JSON, bad CLI arguments and output-write failures produce a short error. Raw inputs cannot be used as the output, including symlink/hard-link aliases. Parent output directories must already exist.

## Reading a comparison

`compareRuns(before, after)` returns `{ comparable, reasons, cases }`. One invocation compares one scenario and returns one case. The case retains both complete raw schema-shaped inputs, including build identities, configuration, completion evidence, checkpoint and integrity fingerprints. An input with an invalid schema shape is represented as null in that raw slot, with explicit field-path reasons. The original raw file remains the evidence.

**Schema version 2.** The probe and this comparator require `schema_version: 2`; `compareRuns` accepts v2↔v2 only and rejects a v1 input on either side with an explicit `schema_version` field-path reason. Retained batch-1 v1 artifacts remain batch-1's evidence; re-pair a baseline by re-running the v2 probe, never by mixing versions. Every v1 field name, the manifest bytes and the fixture/workload hashes are unchanged, and all v1 validation rules still apply. Version 2 adds only measurement fields:

| Field | Meaning |
| --- | --- |
| `backend` | Run-level `{kind: "blob"\|"native_chunks", mode_version: u32\|null}`. Every run today reports `blob` with a null `mode_version`, because the chunked backend does not exist yet. A `blob` run never carries a mode version; a `native_chunks` run must declare one. Two runs of the same kind must agree on `mode_version`; a blob↔native_chunks pair is the intended batch-2 comparison. |
| `samples[*].values.blocks_written` | `u64\|null`, populated on both backends. On the blob backend it is the session's whole block count: that backend re-encodes and rewrites the entire `blocks_json` array on every save, including same-value and final saves. The probe cross-checks it against the after-save fingerprint block count; the comparator checks it against `manifest.initial_blocks_per_session` (no declared workload operation adds or removes a block). |
| `samples[*].values.chunks_written`, `samples[*].values.chunk_bytes_written` | `u64\|null`, null-never-zero. Null exactly while the paired `chunks_written_missing_reason`/`chunk_bytes_written_missing_reason` field says `backend:blob`; a populated counter carries a null reason, and any other combination is rejected. |
| `stages.conversion` | A seventh stage with the ordinary stage shape, after `setup`. Until a workload actually converts a session it stays unattempted with every field null; once attempted it must satisfy the usual completed/timed/counter rules. Its metric paths report `missing_reason: "conversion_not_attempted_by_workload"` for a side that did not convert, so conversion costs stay separate from hot-save improvements. |
| `summary.totals`/`max` new counters | `blocks_written`, `chunks_written`, `chunk_bytes_written`, re-derived from samples like every other total. When no sample supplies a value the aggregate is null, never a fabricated zero. Chunk-counter metric paths report `missing_reason: "backend:blob"` while the lacking side ran the blob backend. |

Schema v2 is a measurement-field revision only: content fingerprints and all cross-run fingerprint equality rules are unchanged from version 1, so a blob↔blob v2 pair compares under the same contract as batch 1.

`cases[0].metrics` uses raw JSON paths, for example `whole_run_process_written_bytes`, `stages.hot.process_written_bytes`, `summary.totals.serialized_bytes`, `summary.max.save_ns` and `samples.0.values.encoding_ns`. Each metric records `before`, `after`, `absolute_difference` (**After − Before**) and `improvement_rate` (**(Before − After) / Before**). Thus 0.9 means a 90% reduction, and a negative value means a regression. Absolute differences remain available when both raw numbers are known, even for incomparable runs; all improvement rates are null unless the entire pair passes validation. A zero Before denominator also gives null. Missing fields/counters give explicit reasons and never become zero.

The comparator requires complete build identity, equal scenario/full manifest (including ordered events and all save parameters), `native-dev`, equal toolchain, native lock hash and all SQLite configuration fields including version/compile-options hash. Source SHA, diff hash and source-tree hash may differ and remain recorded. It requires all stages completed/timed, all process counters known and consistent, no exhausted budget or overshoot, and a final non-BUSY `TRUNCATE` checkpoint with matching log/checkpointed frame counts.

Every declared event must have a successful sample in manifest order with matching event/session/logical time/reason, and `completed_event_indexes` must equal the full manifest event indexes. Sample counts/order must match across runs. Unequal counts never produce paired duration improvements, even if one side supplies a summary or p95. Summary totals/maxima are checked against individual measurements rather than trusted as proof of completion.

All integrity flags must be true. The comparator checks the initial fixture and row/block counts, expected saved rows, retained history/unknown metadata/receipts/queue hashes, restart equality and cross-run fingerprints. A true boolean cannot override disagreeing fingerprints. Version 1 always reports p95, UI response and core lock wait/held timing as null with their missing reasons. Do not invent a p95 for these small fixed workloads; at least 100 explicitly labeled supplemental small samples would require a separate measurement contract and matched counts.

`performance.process_write_target_met` is true/false only for a valid known whole-run process-write reduction with a nonzero Before denominator; otherwise it is null. `performance.enforced` is false: target failure does not turn a complete comparable pair into exit 1. This does not establish UI responsiveness or certify the second batch's write-reduction acceptance.

Keep these quantities separate:

| Quantity | Meaning and limits |
| --- | --- |
| `whole_run_process_written_bytes` | Difference in OS process write counters from main entry through cleanup; includes fixture, setup, verification and temporary-directory cleanup. On macOS the source is `proc_pid_rusage(RUSAGE_INFO_V2).ri_diskio_byteswritten`. It is not physical NAND wear. |
| `process_lifetime_written_bytes` | Final absolute OS counter, used for experiment cost; includes process work before the first baseline read. Never substitute a stage delta for it. |
| `serialized_bytes` | Logical SQL-bound transcript/block/acceptance-queue JSON encodings; excludes transient display/ToolCall/revision encodings. It is not filesystem traffic. |
| `history_bytes_read` | Existing raw transcript buffer reads; no extra reads are introduced by measuring them. |
| `changed_rows` / `commit_count` | Semantic supplied-row changes / actual save commits. A same-value save can have zero changes; inspect raw evidence. |
| `database_bytes` / `wal_bytes` | Per-stage file-size snapshots, not cumulative written bytes. |
| `wal_peak_bytes` | Sampled lower-bound WAL size peak, observed every 2 ms and at operation boundaries. A final empty WAL does not imply no WAL traffic. |
| `save_ns` / encoding/projection/commit timings | Per-save, total and maximum duration measurements. Stage elapsed times include their declared work and waits; stage I/O does not have to sum to whole-run I/O. |

## Measurement status

Batch-1 (2026-11-16) Before/After are finalized by the controller from the retained raw and
comparison evidence. The table's Before/After columns are `stages.hot.process_written_bytes`
(bytes of OS process writes during the hot save stage); 绝对差值 is After − Before; 改善率 is
(Before − After) / Before from the comparator. Raw evidence lives under the gitignored
`.superpowers/sdd/2026-11-16-native-persistence-safety/results/batch-1/` — this table is the
durable in-repo record of the frozen baseline.

| 场景 | Before | After | 绝对差值 | 改善率 | 样本数（Before / After） | 完整性 | 完成状态 |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| append-1m | 6,475,776 | 6,471,680 | −4,096 | 0.000633 | 5 / 5 | 全真 | complete |
| append-10m | 84,746,240 | 84,705,280 | −40,960 | 0.000483 | 5 / 5 | 全真 | complete |
| append-40m | 338,051,072 | 337,985,536 | −65,536 | 0.000194 | 5 / 5 | 全真 | complete |
| same-value | 90,112 | 81,920 | −8,192 | 0.0909 | 5 / 5 | 全真 | complete |
| early-tool | 1,081,344 | 1,081,344 | 0 | 0.0 | 2 / 2 | 全真 | complete |
| large-tool | 135,438,336 | 135,417,856 | −20,480 | 0.000151 | 5 / 5 | 全真 | complete |
| long-answer | 50,954,240 | 50,937,856 | −16,384 | 0.000322 | 5 / 5 | 全真 | complete |
| parallel | 34,566,144 | 34,566,144 | 0 | 0.0 | 20 / 20 | 全真 | complete |

All eight pairs: comparator `comparable: true`, exit 0, one attempt per scenario per side, no
budget exhaustion, final TRUNCATE checkpoint non-BUSY with matching frame counts. Whole-run
process writes for context: Before 803,069,952 B total, After 802,811,904 B total across the
eight runs each side (per-scenario values in the retained comparisons).

Before identity: source_sha `d2675a25c536434e225762fa0482c13b439ac962`, diff_sha256
`43550cb3c6b5142312a7b189183017ed9b9a9628b33ff3ce2d8687c8af7ea7c2`, source_tree_sha256
`d3168a384e32abc631dc1f23d7054824c9ab4ff574e9aa9ebd84e690cb1545d7`. After identity:
source_sha `d2675a25c536434e225762fa0482c13b439ac962` (same commit), diff_sha256
`74f7e06d5c60980ebe906d7732c6d7acf71fa07a6b5b29feba852a29c2ef660a`, source_tree_sha256
`29065ee84687879e1d79f0a30a76b510f5d87cc9310ce2a36117b7159bcc3a77`. Both sides: complete,
toolchain rustc 1.98.1, cargo_lock_sha256
`f9d33039c8563280ad2b21b332267336d717b29389ba571c5aed351751e504bb`, profile native-dev;
source/diff/tree hashes differ as the comparator permits. Each run's `invocation.json` pins
its probe binary SHA256.

Pair write costs (comparator `pair_process_lifetime_written_bytes`, summing both sides' final
absolute counters): 1,605,881,856 B across the eight pairs. Total experiment cost including
all attempts: 1,605,881,856 B over 16 attempts (8 Before + 8 After, one attempt each), zero
unknown counters, zero discarded runs (`experiment-cost.json`). UI lock/response measurement:
待测 (the core probe's UI/p95 fields are null by contract; the fifteen GPUI
persistence-safety tests prove liveness behaviorally, not as a UI timing measurement).
Second-batch 95% write-reduction acceptance: 未达成且不属于本批验收 — batch 1 deliberately
leaves the transcript blob write path unchanged, so hot writes match Before within noise
(0–0.09%); the 95% target belongs to batch 2.

Post-review note: after After collection, the whole-request review's fix round touched the
build script's rerun-if-changed scope, one Inbox save-refusal message, one code comment, and
one log-target constant — none of them alter the probed save path, the fixture, workload,
configuration, lock or toolchain, so the retained pair remains valid evidence; each run's
recorded identity names the exact binary it measured. Before building the probe for a future
measurement, run `touch crates/monocode-core/build.rs` so the embedded identity reflects the
current tree.

To check changes to the comparator, run `node --test scripts/compare-native-persistence.test.mjs`. The tests use synthetic JSON and temporary files only; they run no Rust builds or persistence workload.

## Batch-2 measurement (2026-11-17, native ordinary-session incremental storage)

Re-pairing per the protocol (测量工具或配置改变时重新建立配对基线): the probe changed after the
Task-2 freeze (Task 5 `--backend` flag, Task 6 opt-in/fingerprint reads, Task 8a supplemental
mode), so Before was re-frozen with the **final** binary on the unchanged blob save path and
After uses the **same binary** with `--backend native-chunks` — both sides share one build
identity and differ only in the invocation-level backend flag. The Task-2 freeze runs remain
retained history. Raw evidence: `.superpowers/sdd/2026-11-16-native-transcript-incremental-storage/results/batch-2/runs/`
(gitignored); `compare-task8.json` per scenario.

| 场景 | Before (blob) | After (native) | 绝对差值 | 改善率 | 样本数 | 完整性 | 完成状态 |
| --- | ---: | ---: | ---: | ---: | --- | --- | --- |
| append-1m | 6,471,680 | 143,360 | −6,328,320 | 0.9779 | 5 / 5 | 全真 | complete |
| append-10m | 84,705,280 | 180,224 | −84,525,056 | 0.9979 | 5 / 5 | 全真 | complete |
| append-40m | 337,985,536 | 196,608 | −337,788,928 | 0.9994 | 5 / 5 | 全真 | complete |
| same-value | 81,920 | 65,536 | −16,384 | 0.2000 | 5 / 5 | 全真 | complete |
| early-tool | 1,081,344 | 24,576 | −1,056,768 | 0.9773 | 2 / 2 | 全真 | complete |
| large-tool | 135,417,856 | 86,310,912 | −49,106,944 | 0.3626 | 5 / 5 | 全真 | complete |
| long-answer | 50,937,856 | 344,064 | −50,593,792 | 0.9932 | 5 / 5 | 全真 | complete |
| parallel | 34,545,664 | 868,352 | −33,677,312 | 0.9749 | 20 / 20 | 全真 | complete |

Acceptance lines (spec 验收门槛):

- **既有历史 95% (L124)**: append-40m hot 337,985,536 → 196,608 B = **99.94% 降低** — 达标.
  One attempt per side, no budget exhaustion, all integrity flags true, comparator
  `comparable: true` on all eight pairs.
- **增长关系 (L125)**: per-save logical writes do not grow with completed history — chunk
  bytes `[1,091 / 2,115 / 3,184 / 4,208 / 0]`, history reads `[196 … 8,433]` B and process
  writes 16–49 KB per save are **identical across the 1 MiB, 10 MiB and 40 MiB** After runs.
- **大消息 (L126)**: logical writes per repeated small update are bounded — large-tool saves
  write 2 chunks / ≤8,547 B per save (final same-value save 0 B), long-answer 1 chunk /
  38,742–41,859 B. **诚实标注**: large-tool's *process-level* hot writes are 86,310,912 B
  (36.26% reduction, the weakest scenario) — each save reads ~21 MB (whole-payload read
  amplification, Task 6 concern 1) and the process writes ≈ what it reads (page/WAL
  mechanism unconfirmed); per-save wall time ≈ 220 ms of which projection ≈ 128 ms. The
  stored-format bound is met; the process-I/O gap stays open for the follow-up diagnosis.
- **性能 (L127)**: supplemental controlled run (append-1m shape, 120 labeled small saves,
  native backend): **p95 = 4.95 ms, max = 10.80 ms** (nearest-rank over per-save wall
  nanoseconds; final save excluded). CI has no timing assertions.
- **无变化 (L123)**: same-value final save writes 0 chunk bytes; the scenario's nonzero hot
  totals (81,920 / 65,536 B) are commit/page overhead present on both sides, After lower.
- **一次性转换成本 (L124, 单列)**: not mixed into hot — probe After setup stage reported
  separately per run (e.g. large-tool setup 40,906,752 B); Task 7's direct measurement: an
  82,495 B blob converts writing 164,304 B chunks+payloads ≈ 1.99× for giant members (~1×
  without), plus one full-database consistency backup file per converted session.

Whole-run process writes for context (includes one-time setup on both sides): Before
803,348,480 B, After 438,366,208 B across the eight runs each side (45.43% lower); the
acceptance metric is the hot-stage table above.

Identity — single binary, both sides: source_sha `d2675a25c536434e225762fa0482c13b439ac962`,
diff_sha256 `379fc5f64debdc567a1b8f5f3c92a9f5a17d6b945e3b6a260c260875adc709e7`,
source_tree_sha256 `7163c17d629c5728ebe475cde612dab877219171de68a20f2fcac63ecf087c03`,
toolchain rustc 1.98.1, cargo_lock_sha256
`f9d33039c8563280ad2b21b332267336d717b29389ba571c5aed351751e504bb`, profile native-dev;
each run's `invocation.json` pins the probe binary SHA256 (rebuilt after
`touch crates/monocode-core/build.rs`).

Pair write costs (`pair_process_lifetime_written_bytes`): 1,241,714,688 B across the eight
Task-8 pairs. Batch-2 experiment cost ledger (all attempts, including the Task-2 freeze, the
three Task-6 devchecks, the Task-8 sweep and the supplemental run): 31 attempts, 28 with
known counters summing 2,225,180,672 B, 3 unknown counters (two unretained Task-1 tool
verifications and one controller identity smoke — `null`, never zero) → `total_bytes: null`
(`results/batch-2/experiment-cost.json`).
