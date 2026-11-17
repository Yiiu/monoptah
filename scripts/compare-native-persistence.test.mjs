// Fork-local (Monoptah). Synthetic schema-v2 runs only; no database or Rust build.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { compareRuns } from "./compare-native-persistence.mjs";

const hash = (digit) => digit.repeat(64);
const stageNames = ["setup", "conversion", "hot", "final_save", "checkpoint", "close", "restart"];
const workedStages = stageNames.filter((name) => name !== "conversion");
const metricNames = [
  "history_bytes_read", "serialized_bytes", "changed_rows", "commit_count",
  "projection_ns", "encoding_ns", "commit_ns", "save_ns",
];

// Full producer-shaped data. Values are deliberately small, not measured results.
function run(written = 1000, scenario = "append-1m", mode = "blob") {
  const native = mode === "native_chunks";
  const sessions = scenario === "parallel" ? 4 : 1;
  const hotCount = scenario === "early-tool" ? 1 : 4;
  const operation = {
    "same-value": "same_value", "early-tool": "complete_early_tool",
    "large-tool": "append_tool_input",
  }[scenario] ?? "append_answer";
  const events = [];
  for (let tick = 0; tick < hotCount; tick++) {
    for (let session = 0; session < sessions; session++) {
      events.push({
        index: events.length, logical_ms: tick * 650, session_index: session,
        operation, append_bytes: ["same-value", "early-tool"].includes(scenario) ? 0 : 1024,
        append_sha256: hash("4"), reason: "routine",
      });
    }
  }
  for (let session = 0; session < sessions; session++) {
    events.push({
      index: events.length, logical_ms: 2600, session_index: session,
      operation: "final_save", append_bytes: 0, append_sha256: hash("5"), reason: "final",
    });
  }
  const fingerprint = {
    blocks_sha256: hash("1"), rows_sha256: hash("2"),
    completed_history_sha256: hash("3"), unknown_metadata_sha256: hash("4"),
    receipts_sha256: hash("5"), queue_sha256: hash("6"),
    block_count: 6 * sessions, row_count: 6 * sessions,
  };
  const saved = { ...fingerprint, blocks_sha256: hash("7"), rows_sha256: hash("8") };
  const samples = events.map((event) => ({
    event_index: event.index, session_index: event.session_index, logical_ms: event.logical_ms,
    started_ms: event.logical_ms, completed_ms: event.logical_ms + 1, save_ns: 100,
    io_before: 10, io_after: 10, process_written_bytes: 0,
    values: {
      reason: event.reason, history_bytes_read: 200, serialized_bytes: 100,
      changed_rows: 1, commit_count: 1, lock_wait_ns: null, lock_held_ns: null,
      projection_ns: 10, encoding_ns: 20, commit_ns: 30, succeeded: true,
      blocks_written: native ? 1 : 6,
      chunks_written: native ? 2 : null,
      chunk_bytes_written: native ? 65536 : null,
      chunks_written_missing_reason: native ? null : "backend:blob",
      chunk_bytes_written_missing_reason: native ? null : "backend:blob",
    },
  }));
  const metrics = {
    history_bytes_read: 200, serialized_bytes: 100, changed_rows: 1, commit_count: 1,
    projection_ns: 10, encoding_ns: 20, commit_ns: 30, save_ns: 100,
    lock_wait_ns: null, lock_held_ns: null,
    blocks_written: native ? 1 : 6, chunks_written: native ? 2 : null,
    chunk_bytes_written: native ? 65536 : null,
  };
  const idleConversion = {
    attempted: false, completed: false, elapsed_ns: null,
    io_before: null, io_after: null, process_written_bytes: null,
    database_bytes: null, wal_bytes: null,
  };
  const workedStage = {
    attempted: true, completed: true, elapsed_ns: 100,
    io_before: 10, io_after: 10, process_written_bytes: 0, database_bytes: 4096, wal_bytes: 0,
  };
  return {
    schema_version: 2, scenario, status: "complete", incomplete_reasons: [],
    backend: native
      ? { kind: "native_chunks", mode_version: 1 }
      : { kind: "blob", mode_version: null },
    build: {
      complete: true, source_sha: "a".repeat(40), diff_sha256: hash("a"),
      source_tree_sha256: hash("b"), toolchain: "rustc 1.90.0\nhost: aarch64-apple-darwin\n",
      cargo_lock_sha256: hash("c"), profile: "native-dev",
    },
    sqlite: {
      sqlite_version: "3.50.2", page_size: 4096, journal_mode: "wal", synchronous: 2,
      wal_autocheckpoint: 1000, foreign_keys: 1, busy_timeout_ms: 5000,
      cache_size: -2000, temp_store: 0, locking_mode: "normal", compile_options_sha256: hash("d"),
    },
    manifest: {
      fixture_sha256: hash("1"), fixture_utf8_bytes: 1100000 * sessions,
      completed_history_utf8_bytes_per_session:
        scenario === "append-40m" ? 41943040 : scenario === "append-10m" ? 10485760 : 1048576,
      active_payload_utf8_bytes_per_session: ["large-tool", "long-answer"].includes(scenario) ? 5242880 : 0,
      initial_blocks_per_session: 6, initial_rows_per_session: 6, session_count: sessions,
      fixture_version: 1, workload_sha256: hash("e"), events,
      save_parameters: {
        routine_interval_ms: 650, final_logical_ms: 2600, append_bytes: 1024,
        final_save: "last_message_suffix_even_if_unchanged", database: "new_temporary_blob_schema",
        parallel_sessions: sessions, parallel_policy: "barrier_per_logical_tick_shared_store_mutex",
        busy_timeout_ms: 5000,
      },
    },
    completed_event_indexes: events.map((event) => event.index), samples,
    stages: Object.fromEntries(stageNames.map((name) => [name,
      name === "conversion" ? (native ? { ...workedStage } : idleConversion) : { ...workedStage }])),
    checkpoint: { strategy: "TRUNCATE", busy: 0, log_frames: 0, checkpointed_frames: 0 },
    integrity: {
      before: fingerprint, expected_rows_sha256: hash("8"), after_save: saved,
      after_restart: { ...saved }, initial_fixture_matches: true, rows_match: true,
      completed_history_match: true, unknown_metadata_match: true, receipts_match: true,
      queue_match: true, restart_match: true, sqlite_ok: true,
    },
    whole_run_io_before: 10, whole_run_io_after: 10 + written,
    whole_run_process_written_bytes: written, process_lifetime_written_bytes: 10 + written,
    io_budget_bytes: 536870912, budget_exhausted: false, budget_overshoot_bytes: 0,
    wal_peak_bytes: 4096, wal_peak_method: "sampled_2ms_and_operation_boundaries",
    wall_limit_ms: 120000, elapsed_ns: 3000000000,
    summary: {
      sample_count: events.length, successful_sample_count: events.length,
      totals: Object.fromEntries(Object.entries(metrics).map(([key, value]) =>
        [key, value === null ? null : value * events.length])),
      max: metrics, p95_ns: null, p95_missing_reason: "single_fixed_workload_fewer_than_100_samples",
    },
    ui_response_ns: null, ui_response_missing_reason: "core_probe_has_no_ui",
  };
}
const metric = (result, path) => result.cases[0].metrics[path];
function rejected(before, after, reason) {
  const result = compareRuns(before, after);
  assert.equal(result.comparable, false);
  assert.ok(result.reasons.some((item) => item.includes(reason)), JSON.stringify(result.reasons));
  for (const value of Object.values(result.cases[0].metrics)) assert.equal(value.improvement_rate, null);
  assert.equal(result.cases[0].performance.process_write_target_met, null);
  return result;
}

// Catches denominator/unit mistakes and accidental source-identity equality gates.
test("reports a 0.9 process-write reduction, raw values, difference and separate target", () => {
  const before = run();
  const after = run(100);
  after.build.source_sha = "b".repeat(40);
  after.build.diff_sha256 = hash("c");
  after.build.source_tree_sha256 = hash("d");
  const untouched = structuredClone([before, after]);
  const result = compareRuns(before, after);
  assert.equal(result.comparable, true, JSON.stringify(result.reasons));
  assert.deepEqual(result.reasons, []);
  assert.deepEqual(metric(result, "whole_run_process_written_bytes"), {
    before: 1000, after: 100, absolute_difference: -900, improvement_rate: 0.9, missing_reason: null,
  });
  assert.equal(result.cases[0].performance.process_write_target_met, false);
  assert.equal(result.cases[0].performance.enforced, false);
  assert.equal(result.cases[0].experiment_cost.pair_process_lifetime_written_bytes, 1120);
  assert.deepEqual(result.cases[0].before, before);
  assert.deepEqual(result.cases[0].after, after);
  assert.deepEqual([before, after], untouched);
  assert.equal(metric(result, "summary.totals.serialized_bytes").before, 500);
  assert.equal(metric(result, "wal_peak_bytes").before, 4096);
  assert.equal(metric(result, "samples.0.save_ns").before, 100);
});

test("accepts every fixed scenario, early-tool exception and parallel session order", () => {
  for (const scenario of ["append-1m", "append-10m", "append-40m", "same-value", "early-tool", "large-tool", "long-answer", "parallel"]) {
    const result = compareRuns(run(1000, scenario), run(100, scenario));
    assert.equal(result.comparable, true, `${scenario}: ${result.reasons}`);
  }
});

test("object property order is insignificant but arrays retain order", () => {
  const after = run(100);
  after.manifest = Object.fromEntries(Object.entries(after.manifest).reverse());
  after.sqlite = Object.fromEntries(Object.entries(after.sqlite).reverse());
  assert.equal(compareRuns(run(), after).comparable, true);
  after.completed_event_indexes.reverse();
  rejected(run(), after, "completed_event_indexes");
});

test("rejects fixture, workload, toolchain, lock, profile and every SQLite identity change", () => {
  const changes = [
    ["manifest.fixture_sha256", hash("9")], ["manifest.workload_sha256", hash("9")],
    ["manifest.fixture_utf8_bytes", 1100001], ["manifest.initial_rows_per_session", 7],
    ["build.toolchain", "rustc different"], ["build.cargo_lock_sha256", hash("9")],
    ["build.profile", "release"],
    ...Object.entries({
      sqlite_version: "3.49.0", page_size: 8192, journal_mode: "delete", synchronous: 1,
      wal_autocheckpoint: 2000, foreign_keys: 0, busy_timeout_ms: 4000, cache_size: -3000,
      temp_store: 2, locking_mode: "exclusive", compile_options_sha256: hash("9"),
    }).map(([key, value]) => [`sqlite.${key}`, value]),
  ];
  for (const [path, value] of changes) {
    const after = run(100);
    const [section, key] = path.split(".");
    after[section][key] = value;
    rejected(run(), after, section);
  }
});

test("missing_counts_are_not_zero", () => {
  const after = run(100);
  for (const key of ["whole_run_io_before", "whole_run_io_after", "whole_run_process_written_bytes", "process_lifetime_written_bytes", "budget_overshoot_bytes"]) after[key] = null;
  const result = rejected(run(), after, "whole_run");
  assert.equal(metric(result, "whole_run_process_written_bytes").after, null);
  assert.equal(result.cases[0].experiment_cost.pair_process_lifetime_written_bytes, null);
  const valid = compareRuns(run(), run(100));
  for (const path of ["summary.totals.lock_wait_ns", "summary.max.lock_held_ns", "ui_response_ns", "summary.p95_ns", "samples.0.values.chunks_written", "samples.0.values.chunk_bytes_written", "summary.totals.chunk_bytes_written"]) {
    assert.equal(metric(valid, path).before, null);
    assert.equal(metric(valid, path).improvement_rate, null);
    assert.ok(metric(valid, path).missing_reason);
  }
  // The blob backend has no chunk counters: null with the backend's reason.
  assert.equal(metric(valid, "samples.0.values.chunks_written").missing_reason, "backend:blob");
  assert.equal(metric(valid, "summary.totals.chunk_bytes_written").missing_reason, "backend:blob");
  // An unattempted conversion stage has its own explicit reason.
  assert.equal(metric(valid, "stages.conversion.process_written_bytes").missing_reason, "conversion_not_attempted_by_workload");
});

test("zero_before_has_no_percentage and regressions retain their sign", () => {
  const result = compareRuns(run(0), run(100));
  assert.equal(result.comparable, true);
  assert.equal(metric(result, "whole_run_process_written_bytes").improvement_rate, null);
  assert.equal(metric(result, "whole_run_process_written_bytes").absolute_difference, 100);
  assert.equal(result.cases[0].performance.process_write_target_met, null);
  assert.equal(metric(compareRuns(run(100), run(200)), "whole_run_process_written_bytes").improvement_rate, -1);
  const good = compareRuns(run(1000), run(10));
  assert.equal(good.cases[0].performance.process_write_target_met, true);
});

test("unfinished_or_corrupt_run_is_rejected", () => {
  const changes = [
    (r) => { r.status = "incomplete"; },
    (r) => { r.status = "skipped"; },
    (r) => { r.incomplete_reasons = ["io_budget_exhausted"]; },
    (r) => { r.budget_exhausted = true; },
    (r) => { r.budget_overshoot_bytes = 1; },
    (r) => { r.whole_run_io_after = 536870912; r.process_lifetime_written_bytes = 536870912; r.whole_run_process_written_bytes = 536870902; },
    (r) => { r.io_budget_bytes = 1073741824; },
    (r) => { r.wall_limit_ms = 240000; },
    (r) => { r.elapsed_ns = 120000000001; },
    (r) => { r.integrity.rows_match = false; },
    (r) => { r.integrity.after_restart.rows_sha256 = hash("9"); },
  ];
  for (const change of changes) {
    const after = run(1);
    change(after);
    rejected(run(), after, "after");
  }
});

test("requires every integrity flag and complete build metadata even when both runs agree", () => {
  for (const key of ["initial_fixture_matches", "rows_match", "completed_history_match", "unknown_metadata_match", "receipts_match", "queue_match", "restart_match", "sqlite_ok"]) {
    for (const value of [false, null]) {
      const after = run(100);
      after.integrity[key] = value;
      rejected(run(), after, `integrity.${key}`);
    }
  }
  const before = run();
  const after = run(100);
  before.build.profile = after.build.profile = "release";
  rejected(before, after, "build.profile");
});

test("requires all stages, known consistent counters, WAL observation and final checkpoint", () => {
  for (const name of workedStages) {
    for (const [key, value] of [["attempted", false], ["completed", false], ["elapsed_ns", null], ["io_after", null], ["process_written_bytes", 1]]) {
      const after = run(100);
      after.stages[name][key] = value;
      rejected(run(), after, `stages.${name}`);
    }
  }
  // The conversion stage is optional until a workload converts a session, but
  // it must stay entirely empty while unattempted and fully valid once run.
  for (const change of [
    (s) => { s.attempted = true; },
    (s) => { s.completed = true; },
    (s) => { s.elapsed_ns = 100; },
    (s) => { s.io_before = 10; },
    (s) => { s.process_written_bytes = 0; },
    (s) => { s.database_bytes = 4096; },
  ]) {
    const after = run(100);
    change(after.stages.conversion);
    rejected(run(), after, "stages.conversion");
  }
  const unfinished = run(100, "append-1m", "native_chunks");
  unfinished.stages.conversion.completed = false;
  rejected(run(100, "append-1m", "native_chunks"), unfinished, "stages.conversion");
  for (const [key, value] of [["strategy", "PASSIVE"], ["busy", 1], ["log_frames", 2], ["checkpointed_frames", null]]) {
    const after = run(100);
    after.checkpoint[key] = value;
    rejected(run(), after, "checkpoint");
  }
  for (const [key, value] of [["wal_peak_bytes", null], ["whole_run_process_written_bytes", 1], ["process_lifetime_written_bytes", 0], ["whole_run_io_before", 99999]]) {
    const after = run(100);
    after[key] = value;
    rejected(run(), after, key === "wal_peak_bytes" ? "wal_peak" : "whole_run");
  }
});

test("requires every event and successful complete sample values; never pairs unequal counts", () => {
  const changes = [
    (r) => { r.completed_event_indexes.pop(); },
    (r) => { r.samples.pop(); },
    (r) => { r.samples.reverse(); },
    (r) => { r.samples[0].session_index = 1; },
    (r) => { r.samples[0].logical_ms = 1; },
    (r) => { r.samples[0].completed_ms = -1; },
    (r) => { r.samples[0].values.reason = "final"; },
    (r) => { r.samples[0].values.succeeded = false; },
    (r) => { r.samples[0].values = null; },
    (r) => { r.samples[0].io_after = null; },
    (r) => { r.samples[0].process_written_bytes = 1; },
    (r) => { r.manifest.events[0].logical_ms = 1; },
    (r) => { r.manifest.events[0].append_bytes = 1; },
    (r) => { r.manifest.save_parameters.routine_interval_ms = 100; },
  ];
  for (const change of changes) {
    const after = run(100);
    change(after);
    const result = rejected(run(), after, "after");
    assert.equal(metric(result, "samples.0.save_ns").improvement_rate, null);
    assert.equal(metric(result, "summary.totals.save_ns").improvement_rate, null);
  }
});

test("does not trust integrity booleans or summary totals without checking raw evidence", () => {
  const changes = [
    (r) => { r.integrity.before.blocks_sha256 = hash("9"); },
    (r) => { r.integrity.before.row_count++; },
    (r) => { r.integrity.expected_rows_sha256 = hash("9"); },
    (r) => { r.integrity.after_save.queue_sha256 = hash("9"); },
    (r) => { r.summary.sample_count++; },
    (r) => { r.summary.successful_sample_count--; },
    (r) => { r.summary.totals.serialized_bytes++; },
    (r) => { r.summary.max.save_ns++; },
    (r) => { r.summary.p95_ns = 100; },
    (r) => { r.summary.totals.blocks_written++; },
    (r) => { r.summary.totals.chunks_written = 5; },
    (r) => { r.summary.max.chunks_written = 5; },
    (r) => { r.summary.max.blocks_written = 9; },
    (r) => { r.samples[0].values.blocks_written = 5; },
    (r) => { r.samples[0].values.blocks_written = null; },
    (r) => { r.samples[0].values.chunks_written = 3; },
    (r) => { delete r.samples[0].values.chunks_written_missing_reason; },
  ];
  for (const change of changes) {
    const after = run(100);
    change(after);
    rejected(run(), after, "after");
  }
  // Internally consistent fingerprints can still disagree between runs.
  const after = run(100);
  for (const point of ["before", "after_save", "after_restart"]) after.integrity[point].receipts_sha256 = hash("9");
  rejected(run(), after, "integrity");
});

test("rejects missing fields, wrong shapes, unsafe numbers and unknown schema without throwing", () => {
  for (const bad of [null, [], "private-body-marker", {}, { schema_version: 1 }, { schema_version: 2 }]) rejected(run(), bad, "after");
  const changes = [
    (r) => { delete r.build.diff_sha256; },
    (r) => { r.build.complete = false; },
    (r) => { r.build.source_sha = "unknown"; },
    (r) => { r.build.toolchain = null; },
    (r) => { r.sqlite.extra_private_field = "private-body-marker"; },
    (r) => { r.manifest = null; },
    (r) => { r.whole_run_io_after = Number.MAX_SAFE_INTEGER + 1; },
    (r) => { r.samples[0].values.serialized_bytes = "100"; },
    (r) => { delete r.samples[0].values.lock_wait_ns; },
    (r) => { r.samples[0].save_ns = Infinity; },
    (r) => { r.ui_response_ns = 0; },
    (r) => { delete r.backend; },
    (r) => { r.backend.kind = "fork"; },
    (r) => { r.backend.mode_version = -1; },
    (r) => { r.backend.mode_version = 1.5; },
    (r) => { delete r.stages.conversion; },
    (r) => { delete r.samples[0].values.blocks_written; },
  ];
  for (const change of changes) {
    const after = run(100);
    change(after);
    const result = rejected(run(), after, "after");
    assert.ok(!result.reasons.join(" ").includes("private-body-marker"));
  }
});

test("exposes totals, maxima, stage sizes and sample measurements without mixing units", () => {
  const result = compareRuns(run(), run(100));
  for (const key of [...metricNames, "blocks_written", "chunks_written", "chunk_bytes_written"]) {
    assert.ok(metric(result, `summary.totals.${key}`));
    assert.ok(metric(result, `summary.max.${key}`));
  }
  for (const stage of stageNames) {
    assert.equal(metric(result, `stages.${stage}.process_written_bytes`).before, stage === "conversion" ? null : 0);
    assert.equal(metric(result, `stages.${stage}.database_bytes`).before, stage === "conversion" ? null : 4096);
  }
  assert.equal(metric(result, "summary.totals.commit_count").before, 5);
  assert.equal(metric(result, "samples.4.values.encoding_ns").before, 20);
  assert.equal(metric(result, "summary.totals.blocks_written").before, 30);
  assert.equal(metric(result, "samples.0.values.blocks_written").before, 6);
});

test("v2 only: a v1 input is rejected on either side with schema_version field-path reasons", () => {
  const v1 = (written) => {
    const r = run(written);
    r.schema_version = 1;
    delete r.backend;
    delete r.stages.conversion;
    for (const s of r.samples) {
      delete s.values.blocks_written;
      delete s.values.chunks_written;
      delete s.values.chunk_bytes_written;
      delete s.values.chunks_written_missing_reason;
      delete s.values.chunk_bytes_written_missing_reason;
    }
    for (const kind of ["totals", "max"]) {
      for (const key of ["blocks_written", "chunks_written", "chunk_bytes_written"]) delete r.summary[kind][key];
    }
    return r;
  };
  rejected(v1(), run(100), "before.schema_version");
  rejected(run(), v1(100), "after.schema_version");
  rejected(v1(), v1(100), "schema_version");
});

test("accepts the batch-2 shape: a blob before paired with a native_chunks after", () => {
  const result = compareRuns(run(), run(100, "append-1m", "native_chunks"));
  assert.equal(result.comparable, true, JSON.stringify(result.reasons));
  assert.deepEqual(result.reasons, []);
  assert.equal(result.cases[0].before.backend.kind, "blob");
  assert.equal(result.cases[0].after.backend.kind, "native_chunks");
  assert.deepEqual(metric(result, "samples.0.values.chunks_written"), {
    before: null, after: 2, absolute_difference: null, improvement_rate: null, missing_reason: "backend:blob",
  });
  assert.deepEqual(metric(result, "samples.0.values.blocks_written"), {
    before: 6, after: 1, absolute_difference: -5, improvement_rate: 5 / 6, missing_reason: null,
  });
  assert.deepEqual(metric(result, "summary.totals.chunks_written"), {
    before: null, after: 10, absolute_difference: null, improvement_rate: null, missing_reason: "backend:blob",
  });
  assert.equal(metric(result, "stages.conversion.process_written_bytes").before, null);
  assert.equal(metric(result, "stages.conversion.process_written_bytes").after, 0);
  // The blob side never converts, so conversion costs cannot pair; they are
  // reported separately, never as a fabricated improvement.
  assert.equal(metric(result, "stages.conversion.process_written_bytes").missing_reason, "conversion_not_attempted_by_workload");
  assert.equal(metric(result, "stages.conversion.process_written_bytes").absolute_difference, null);
});

test("rejects backend mode lies, same-kind mode mismatches and native reason lies", () => {
  const bump = run(100, "append-1m", "native_chunks");
  bump.backend.mode_version = 2;
  rejected(run(100, "append-1m", "native_chunks"), bump, "pair.backend.mode_version");
  const blobMode = run(100);
  blobMode.backend.mode_version = 1;
  rejected(run(), blobMode, "backend.mode_version");
  const nativeNull = run(100, "append-1m", "native_chunks");
  nativeNull.backend.mode_version = null;
  rejected(run(100, "append-1m", "native_chunks"), nativeNull, "backend.mode_version");
  const reasonLie = run(100, "append-1m", "native_chunks");
  reasonLie.samples[0].values.chunks_written_missing_reason = "backend:blob";
  rejected(run(100, "append-1m", "native_chunks"), reasonLie, "chunks_written");
});

test("refuses to pair a supplemental run on either side, from manifest or report label", () => {
  // A supplemental run carries both labels: the manifest markers and the
  // top-level report object. Its workload is not any fixed scenario, so no
  // pairing is attempted and no improvement rate is ever computed.
  const supplement = (label) => {
    const supplemental = run(100);
    if (label === "manifest" || label === "both") {
      supplemental.manifest.supplemental = true;
      supplemental.manifest.supplemental_saves = 120;
    }
    if (label === "report" || label === "both") {
      supplemental.supplemental = {
        label: true, samples: 120, p95_ns: 1000, max_ns: 9000,
        definition: "nearest-rank p95 of samples[].save_ns over the supplemental routine small-append saves",
      };
    }
    return supplemental;
  };
  const expectReason = "supplemental small-save run is not pair material";
  for (const [before, after, path] of [
    [run(), supplement("both"), "pair.after.supplemental"],
    [supplement("both"), run(100), "pair.before.supplemental"],
    [run(), supplement("manifest"), "pair.after.supplemental"],
    [supplement("manifest"), run(100), "pair.before.supplemental"],
    [run(), supplement("report"), "pair.after.supplemental"],
    [supplement("report"), run(100), "pair.before.supplemental"],
  ]) {
    const result = rejected(before, after, path);
    assert.ok(result.reasons.includes(`${path}: ${expectReason}`), JSON.stringify(result.reasons));
  }
  // Two supplemental runs never pair each other either.
  rejected(supplement("both"), structuredClone(supplement("both")), "not pair material");
});

test("CLI writes comparable or rejected reports, preserves raw inputs and handles failures privately", () => {
  const dir = mkdtempSync(join(tmpdir(), "persistence-compare-"));
  const before = join(dir, "before.json");
  const after = join(dir, "after.json");
  const output = join(dir, "comparison.json");
  const script = fileURLToPath(new URL("./compare-native-persistence.mjs", import.meta.url));
  const invoke = (args) => spawnSync(process.execPath, [script, ...args], { encoding: "utf8" });
  const args = ["--before", before, "--after", after, "--out", output];
  try {
    writeFileSync(before, JSON.stringify(run()));
    writeFileSync(after, JSON.stringify(run(100)));
    let cli = invoke(args);
    assert.equal(cli.status, 0, cli.stderr);
    assert.equal(cli.stdout, "");
    assert.equal(cli.stderr, "");
    assert.equal(JSON.parse(readFileSync(output, "utf8")).comparable, true);
    assert.equal(JSON.parse(readFileSync(before, "utf8")).whole_run_process_written_bytes, 1000);
    const bad = run(100);
    bad.status = "incomplete";
    writeFileSync(after, JSON.stringify(bad));
    cli = invoke(args);
    assert.equal(cli.status, 1);
    assert.equal(JSON.parse(readFileSync(output, "utf8")).comparable, false);
    writeFileSync(after, '{"private-body-marker": invalid}');
    for (const badArgs of [[], args.slice(0, 5), [...args, "--unexpected", "private-body-marker"], [...args, "--out", output], ["--before", join(dir, "missing"), "--after", after, "--out", output], args]) {
      cli = invoke(badArgs);
      assert.equal(cli.status, 1);
      assert.equal(cli.stdout, "");
      assert.ok(cli.stderr.length > 0);
      assert.doesNotMatch(cli.stderr, /private-body-marker|SyntaxError|Error:|\n\s+at /);
    }
    writeFileSync(after, JSON.stringify(run(100)));
    for (const target of [before, after, join(dir, "missing-parent", "out.json")]) {
      cli = invoke(["--before", before, "--after", after, "--out", target]);
      assert.equal(cli.status, 1);
      assert.doesNotMatch(cli.stderr, /\n\s+at /);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
