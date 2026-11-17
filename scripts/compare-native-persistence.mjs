// Fork-local (Monoptah). Compares two transcript_save_probe schema-v2 results.
import { readFileSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const STAGES = ["setup", "conversion", "hot", "final_save", "checkpoint", "close", "restart"];
const SCENARIOS = ["append-1m", "append-10m", "append-40m", "same-value", "early-tool", "large-tool", "long-answer", "parallel"];
const METRICS = ["history_bytes_read", "serialized_bytes", "changed_rows", "commit_count", "projection_ns", "encoding_ns", "commit_ns", "save_ns", "lock_wait_ns", "lock_held_ns", "blocks_written", "chunks_written", "chunk_bytes_written"];
// Chunk counters exist only on the chunked backend; null carries the blob reason.
const CHUNK_COUNTERS = ["chunks_written", "chunk_bytes_written"];
const NEW_COUNTERS = ["blocks_written", "chunks_written", "chunk_bytes_written"];
const INTEGRITY_FLAGS = ["initial_fixture_matches", "rows_match", "completed_history_match", "unknown_metadata_match", "receipts_match", "queue_match", "restart_match", "sqlite_ok"];
const RETAINED_HASHES = ["completed_history_sha256", "unknown_metadata_sha256", "receipts_sha256", "queue_sha256"];
const BUDGET = 536870912;
const WALL_LIMIT_MS = 120000;
const isObject = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
const isCount = (v) => Number.isSafeInteger(v) && v >= 0;
const issue = (reasons, path, message) => reasons.push(`${path}: ${message}`);

// The validators name only known schema paths, never input values or unknown keys.
const field = (predicate, description) => (value, path, reasons) => {
  if (!predicate(value)) issue(reasons, path, `expected ${description}`);
};
const count = field(isCount, "nonnegative safe integer");
const integer = field(Number.isSafeInteger, "safe integer");
const text = field((v) => typeof v === "string" && v.trim().length > 0, "nonempty string");
const hash = field((v) => typeof v === "string" && /^[a-f0-9]{64}$/.test(v), "lowercase SHA-256");
const boolean = field((v) => typeof v === "boolean", "boolean");
const literal = (expected) => field((v) => v === expected, "fixed constant");
const nullable = (rule) => (v, path, reasons) => { if (v !== null) rule(v, path, reasons); };
const array = (rule) => (v, path, reasons) => {
  if (!Array.isArray(v)) return issue(reasons, path, "expected array");
  for (let i = 0; i < v.length; i++) rule(v[i], `${path}.${i}`, reasons);
};
const object = (fields) => (v, path, reasons) => {
  if (!isObject(v)) return issue(reasons, path, "expected object");
  if (Object.keys(v).some((key) => !Object.hasOwn(fields, key))) issue(reasons, path, "unexpected fields");
  for (const [key, rule] of Object.entries(fields)) rule(v[key], `${path}.${key}`, reasons);
};
const reason = field((v) => v === "routine" || v === "final", "save reason");
const backendReason = field((v) => v === "backend:blob", "backend:blob reason");
const metricFields = Object.fromEntries(METRICS.map((key) => [key,
  key.startsWith("lock_") ? literal(null) : CHUNK_COUNTERS.includes(key) ? nullable(count) : count,
]));
const backend = object({
  kind: field((v) => v === "blob" || v === "native_chunks", "storage backend kind"),
  mode_version: nullable(field((v) => Number.isSafeInteger(v) && v >= 0 && v <= 4294967295, "u32 mode version")),
});
const fingerprint = object({
  blocks_sha256: hash, rows_sha256: hash,
  ...Object.fromEntries(RETAINED_HASHES.map((key) => [key, hash])),
  block_count: count, row_count: count,
});
const event = object({
  index: count, logical_ms: count, session_index: count,
  operation: field((v) => ["append_answer", "append_tool_input", "complete_early_tool", "same_value", "final_save"].includes(v), "event operation"),
  append_bytes: count, append_sha256: hash, reason,
});
const sample = object({
  event_index: count, session_index: count, logical_ms: count, started_ms: count,
  completed_ms: count, save_ns: count, io_before: nullable(count), io_after: nullable(count),
  process_written_bytes: nullable(count), values: nullable(object({
    ...Object.fromEntries(Object.entries(metricFields).filter(([key]) => key !== "save_ns")),
    reason, succeeded: boolean,
    chunks_written_missing_reason: nullable(backendReason),
    chunk_bytes_written_missing_reason: nullable(backendReason),
  })),
});
const stage = object({
  attempted: boolean, completed: boolean, elapsed_ns: nullable(count),
  io_before: nullable(count), io_after: nullable(count), process_written_bytes: nullable(count),
  database_bytes: nullable(count), wal_bytes: nullable(count),
});
const runSchema = object({
  schema_version: literal(2),
  scenario: nullable(field((v) => SCENARIOS.includes(v), "scenario ID")),
  status: field((v) => ["complete", "incomplete", "skipped"].includes(v), "run status"),
  incomplete_reasons: array(field((v) => typeof v === "string" && /^[a-z][a-z0-9_]*$/.test(v), "fixed reason code")),
  build: nullable(object({
    complete: boolean,
    source_sha: nullable(field((v) => typeof v === "string" && /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(v), "Git commit ID")),
    diff_sha256: hash, source_tree_sha256: hash, toolchain: nullable(text),
    cargo_lock_sha256: nullable(hash), profile: nullable(text),
  })),
  sqlite: nullable(object({
    sqlite_version: text, page_size: count, journal_mode: text, synchronous: count,
    wal_autocheckpoint: count, foreign_keys: count, busy_timeout_ms: count,
    cache_size: integer, temp_store: count, locking_mode: text, compile_options_sha256: hash,
  })),
  backend,
  manifest: nullable(object({
    fixture_sha256: hash, fixture_utf8_bytes: count, completed_history_utf8_bytes_per_session: count,
    active_payload_utf8_bytes_per_session: count, initial_blocks_per_session: count,
    initial_rows_per_session: count, session_count: count, fixture_version: literal(1),
    workload_sha256: hash, events: array(event), save_parameters: object({
      routine_interval_ms: literal(650), final_logical_ms: literal(2600), append_bytes: literal(1024),
      final_save: literal("last_message_suffix_even_if_unchanged"), database: literal("new_temporary_blob_schema"),
      parallel_sessions: count, parallel_policy: literal("barrier_per_logical_tick_shared_store_mutex"),
      busy_timeout_ms: literal(5000),
    }),
  })),
  completed_event_indexes: array(count), samples: array(sample),
  stages: object(Object.fromEntries(STAGES.map((key) => [key, stage]))),
  checkpoint: object({ strategy: literal("TRUNCATE"), busy: nullable(integer), log_frames: nullable(integer), checkpointed_frames: nullable(integer) }),
  integrity: object({
    before: nullable(fingerprint), after_save: nullable(fingerprint), after_restart: nullable(fingerprint),
    expected_rows_sha256: nullable(hash),
    ...Object.fromEntries(INTEGRITY_FLAGS.map((key) => [key, nullable(boolean)])),
  }),
  whole_run_io_before: nullable(count), whole_run_io_after: nullable(count),
  whole_run_process_written_bytes: nullable(count), process_lifetime_written_bytes: nullable(count),
  io_budget_bytes: literal(BUDGET), budget_exhausted: boolean, budget_overshoot_bytes: nullable(count),
  wal_peak_bytes: nullable(count), wal_peak_method: literal("sampled_2ms_and_operation_boundaries"),
  wall_limit_ms: literal(WALL_LIMIT_MS), elapsed_ns: count,
  summary: nullable(object({
    sample_count: count, successful_sample_count: count,
    totals: nullable(object(metricFields)),
    max: nullable(object(Object.fromEntries(METRICS.map((key) => [key, nullable(metricFields[key])])))),
    p95_ns: literal(null), p95_missing_reason: literal("single_fixed_workload_fewer_than_100_samples"),
  })),
  ui_response_ns: literal(null), ui_response_missing_reason: literal("core_probe_has_no_ui"),
});

// JSON objects have no significant key order. Arrays (events/samples) do.
function same(a, b) {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((v, i) => same(v, b[i]));
  if (!isObject(a) || !isObject(b)) return false;
  const keys = Object.keys(a);
  return keys.length === Object.keys(b).length && keys.every((key) => Object.hasOwn(b, key) && same(a[key], b[key]));
}

function validateRun(run, side, reasons) {
  const start = reasons.length;
  runSchema(run, side, reasons);
  if (reasons.length !== start) return false;
  const check = (ok, path, message) => { if (!ok) issue(reasons, `${side}.${path}`, message); };
  const equal = (a, b, path) => check(same(a, b), path, "inconsistent evidence");
  const counters = (a, b, delta, path) => {
    check([a, b, delta].every(isCount), path, "process counters unavailable");
    if ([a, b, delta].every(isCount)) {
      check(b >= a && b - a === delta, path, "process counter delta inconsistent");
      check(a >= run.whole_run_io_before && b <= run.whole_run_io_after, path, "outside whole_run counters");
    }
  };
  check(run.status === "complete", "status", "run not complete");
  check(run.incomplete_reasons.length === 0, "incomplete_reasons", "run has incomplete reasons");
  check(run.scenario !== null, "scenario", "scenario unavailable");
  check(run.build?.complete === true, "build.complete", "build identity incomplete");
  for (const key of ["source_sha", "toolchain", "cargo_lock_sha256"]) check(run.build?.[key] != null, `build.${key}`, "build identity unavailable");
  check(run.build?.profile === "native-dev", "build.profile", "requires native-dev");
  check(run.sqlite !== null, "sqlite", "configuration unavailable");
  if (run.sqlite) {
    const db = run.sqlite;
    check(db.page_size >= 512 && db.page_size <= 65536 && (db.page_size & (db.page_size - 1)) === 0, "sqlite.page_size", "invalid page size");
    check(db.journal_mode === "wal" && db.synchronous >= 1 && db.synchronous <= 3 && db.wal_autocheckpoint > 0, "sqlite", "requires durable WAL configuration");
    check(db.foreign_keys === 1 && db.busy_timeout_ms === 5000 && db.temp_store <= 2 && ["normal", "exclusive"].includes(db.locking_mode), "sqlite", "invalid connection configuration");
  }
  // The blob backend has no mode version; a chunked run must declare one.
  if (run.backend.kind === "blob") check(run.backend.mode_version === null, "backend.mode_version", "blob backend has no mode version");
  else check(run.backend.mode_version !== null, "backend.mode_version", "native backend must declare its mode version");
  counters(run.whole_run_io_before, run.whole_run_io_after, run.whole_run_process_written_bytes, "whole_run");
  check(isCount(run.process_lifetime_written_bytes) && run.process_lifetime_written_bytes === run.whole_run_io_after, "whole_run.process_lifetime_written_bytes", "final absolute counter inconsistent or unavailable");
  check(isCount(run.process_lifetime_written_bytes) && run.process_lifetime_written_bytes < BUDGET, "whole_run.process_lifetime_written_bytes", "absolute process budget exhausted or unavailable");
  check(!run.budget_exhausted && run.budget_overshoot_bytes === 0, "budget_overshoot_bytes", "budget exhausted, overshot or unobserved");
  check(run.elapsed_ns <= WALL_LIMIT_MS * 1000000, "elapsed_ns", "wall time limit exceeded");
  check(isCount(run.wal_peak_bytes), "wal_peak_bytes", "WAL peak unavailable");
  for (const name of STAGES) {
    const s = run.stages[name];
    if (name === "conversion" && !s.attempted) {
      // Until a workload converts a session the stage stays entirely empty.
      check(s.completed === false && [s.elapsed_ns, s.io_before, s.io_after, s.process_written_bytes, s.database_bytes, s.wal_bytes].every((v) => v === null), "stages.conversion", "unattempted conversion stage must stay empty");
      continue;
    }
    check(s.attempted && s.completed && isCount(s.elapsed_ns), `stages.${name}`, "stage not completed or timed");
    counters(s.io_before, s.io_after, s.process_written_bytes, `stages.${name}`);
  }
  const cp = run.checkpoint;
  check(cp.busy === 0 && isCount(cp.log_frames) && isCount(cp.checkpointed_frames) && cp.log_frames === cp.checkpointed_frames, "checkpoint", "final checkpoint failed, BUSY or unavailable");

  const m = run.manifest;
  check(m !== null, "manifest", "manifest unavailable");
  if (m) {
    const sessions = run.scenario === "parallel" ? 4 : 1;
    const hotCount = run.scenario === "early-tool" ? 1 : 4;
    const history = run.scenario === "append-40m" ? 41943040 : run.scenario === "append-10m" ? 10485760 : 1048576;
    equal(m.completed_history_utf8_bytes_per_session, history, "manifest.completed_history_utf8_bytes_per_session");
    equal(m.active_payload_utf8_bytes_per_session, ["large-tool", "long-answer"].includes(run.scenario) ? 5242880 : 0, "manifest.active_payload_utf8_bytes_per_session");
    check(m.fixture_utf8_bytes > 0 && m.initial_blocks_per_session > 0 && m.initial_rows_per_session > 0, "manifest", "empty fixture");
    equal(m.session_count, sessions, "manifest.session_count");
    equal(m.save_parameters.parallel_sessions, sessions, "manifest.save_parameters.parallel_sessions");
    equal(m.events.length, (hotCount + 1) * sessions, "manifest.events");
    const operation = { "same-value": "same_value", "early-tool": "complete_early_tool", "large-tool": "append_tool_input" }[run.scenario] ?? "append_answer";
    for (const [i, e] of m.events.entries()) {
      const final = i >= hotCount * sessions;
      const expected = {
        index: i, session_index: i % sessions,
        logical_ms: final ? 2600 : Math.floor(i / sessions) * 650,
        operation: final ? "final_save" : operation, reason: final ? "final" : "routine",
        append_bytes: final || ["same-value", "early-tool"].includes(run.scenario) ? 0 : 1024,
      };
      for (const [key, value] of Object.entries(expected)) equal(e[key], value, `manifest.events.${i}.${key}`);
    }
    equal(run.completed_event_indexes, m.events.map((e) => e.index), "completed_event_indexes");
    equal(run.samples.length, m.events.length, "samples.length");
  }
  for (const [i, s] of run.samples.entries()) {
    const path = `samples.${i}`;
    const e = m?.events[i];
    check(e != null, path, "sample has no declared event");
    if (e) {
      equal([s.event_index, s.session_index, s.logical_ms, s.values?.reason], [e.index, e.session_index, e.logical_ms, e.reason], path);
    }
    check(s.started_ms >= s.logical_ms && s.completed_ms >= s.started_ms, path, "invalid event timing");
    check(s.values?.succeeded === true, `${path}.values`, "sample failed or values unavailable");
    // blocks_written is populated on both backends; a null chunk counter must
    // carry exactly the blob backend's missing reason, never none.
    check(isCount(s.values?.blocks_written), `${path}.values.blocks_written`, "blocks written unavailable");
    for (const key of CHUNK_COUNTERS) {
      const missing = s.values?.[key] == null;
      check(missing ? s.values?.[`${key}_missing_reason`] === "backend:blob" : s.values?.[`${key}_missing_reason`] == null,
        `${path}.values.${key}`, "chunk counter and missing reason disagree");
    }
    if (run.backend.kind === "blob" && m) {
      // No declared workload operation adds or removes a block, so the blob
      // backend's per-save width is the session's whole block count.
      equal(s.values?.blocks_written, m.initial_blocks_per_session, `${path}.values.blocks_written`);
    }
    counters(s.io_before, s.io_after, s.process_written_bytes, path);
  }
  const integrity = run.integrity;
  for (const key of INTEGRITY_FLAGS) check(integrity[key] === true, `integrity.${key}`, "integrity not verified");
  for (const key of ["before", "after_save", "after_restart"]) check(integrity[key] !== null, `integrity.${key}`, "fingerprint unavailable");
  check(integrity.expected_rows_sha256 !== null, "integrity.expected_rows_sha256", "expected rows unavailable");
  if (integrity.before && m) {
    equal(integrity.before.blocks_sha256, m.fixture_sha256, "integrity.before.blocks_sha256");
    equal(integrity.before.block_count, m.initial_blocks_per_session * m.session_count, "integrity.before.block_count");
    equal(integrity.before.row_count, m.initial_rows_per_session * m.session_count, "integrity.before.row_count");
  }
  if (integrity.after_save && integrity.before) {
    equal(integrity.after_save.rows_sha256, integrity.expected_rows_sha256, "integrity.after_save.rows_sha256");
    for (const key of RETAINED_HASHES) equal(integrity.after_save[key], integrity.before[key], `integrity.after_save.${key}`);
  }
  equal(integrity.after_restart, integrity.after_save, "integrity.after_restart");
  check(run.summary !== null, "summary", "summary unavailable");
  if (run.summary) {
    equal(run.summary.sample_count, run.samples.length, "summary.sample_count");
    equal(run.summary.successful_sample_count, run.samples.filter((s) => s.values?.succeeded).length, "summary.successful_sample_count");
    for (const kind of ["totals", "max"]) {
      check(run.summary[kind] !== null, `summary.${kind}`, "measurements unavailable");
      if (!run.summary[kind]) continue;
      for (const key of METRICS) {
        const values = run.samples.map((s) => key === "save_ns" ? s.save_ns : s.values?.[key]).filter(isCount);
        const expected = key.startsWith("lock_") ? null
          : NEW_COUNTERS.includes(key) && !values.length ? null
          : kind === "totals"
          ? values.reduce((sum, value) => sum + value, 0) : values.length ? Math.max(...values) : null;
        check(expected === null || isCount(expected), `summary.${kind}.${key}`, "aggregate exceeds safe integer range");
        equal(run.summary[kind][key], expected, `summary.${kind}.${key}`);
      }
    }
  }
  return true; // Shape is valid; semantic failures are retained in reasons.
}

const get = (run, path) => path.split(".").reduce((v, key) => v?.[key], run);
function measurement(before, after, path, comparable) {
  const a = get(before, path);
  const b = get(after, path);
  const known = isCount(a) && isCount(b);
  // A chunk counter absent because that side ran the blob backend, and an
  // unattempted conversion stage, carry their own explicit reasons.
  const blobMissing = CHUNK_COUNTERS.some((key) => path.endsWith(`.${key}`))
    && [[before, a], [after, b]].some(([run, value]) => value == null && get(run, "backend.kind") === "blob");
  const conversionIdle = path.startsWith("stages.conversion")
    && [[before, a], [after, b]].some(([run, value]) => value == null && !get(run, "stages.conversion.attempted"));
  const missing = path.includes("lock_") ? "core_probe_has_no_lock_timing"
    : path === "ui_response_ns" ? "core_probe_has_no_ui"
      : path === "summary.p95_ns" ? "single_fixed_workload_fewer_than_100_samples"
        : blobMissing ? "backend:blob"
          : conversionIdle ? "conversion_not_attempted_by_workload" : "measurement_unavailable";
  return {
    before: isCount(a) ? a : null, after: isCount(b) ? b : null,
    absolute_difference: known ? b - a : null,
    improvement_rate: comparable && known && a > 0 ? (a - b) / a : null,
    missing_reason: !known ? missing : !comparable ? "runs_not_comparable" : a === 0 ? "zero_before_denominator" : null,
  };
}

export function compareRuns(before, after) {
  const reasons = [];
  // A supplemental run (`--supplemental-saves`) is a labeled sample-collection
  // mode: its workload and cadence match no fixed scenario, so it is never
  // pair material, whichever side declares it and however valid it otherwise
  // is. The label is honored from the manifest or the report object.
  for (const [side, sideRun] of [["before", before], ["after", after]]) {
    if (sideRun?.manifest?.supplemental === true || isObject(sideRun?.supplemental)) {
      issue(reasons, `pair.${side}.supplemental`, "supplemental small-save run is not pair material");
    }
  }
  const beforeShape = validateRun(before, "before", reasons);
  const afterShape = validateRun(after, "after", reasons);
  if (beforeShape && afterShape) {
    for (const path of [
      "scenario", "manifest", "sqlite", "build.toolchain", "build.cargo_lock_sha256", "build.profile",
      "integrity.before", "integrity.expected_rows_sha256", "integrity.after_save", "integrity.after_restart",
    ]) if (!same(get(before, path), get(after, path))) issue(reasons, `pair.${path}`, "mismatch");
    // A blob/native_chunks pair is the intended treatment; two runs of the
    // same kind must still agree on their mode version.
    if (before.backend.kind === after.backend.kind && !same(before.backend.mode_version, after.backend.mode_version)) {
      issue(reasons, "pair.backend.mode_version", "mismatch");
    }
    const identity = (run) => run.samples.map((s) => [s.event_index, s.session_index, s.logical_ms, s.values?.reason]);
    if (!same(identity(before), identity(after))) issue(reasons, "pair.samples", "sample count or event order mismatch");
  }
  const comparable = reasons.length === 0;
  const paths = ["whole_run_process_written_bytes", "elapsed_ns", "wal_peak_bytes", "ui_response_ns", "summary.p95_ns"];
  for (const stage of STAGES) for (const key of ["elapsed_ns", "process_written_bytes", "database_bytes", "wal_bytes"]) paths.push(`stages.${stage}.${key}`);
  for (const kind of ["totals", "max"]) for (const key of METRICS) paths.push(`summary.${kind}.${key}`);
  const sampleCount = Math.max(Array.isArray(before?.samples) ? before.samples.length : 0, Array.isArray(after?.samples) ? after.samples.length : 0);
  for (let i = 0; i < sampleCount; i++) {
    for (const key of ["save_ns", "process_written_bytes"]) paths.push(`samples.${i}.${key}`);
    for (const key of METRICS.filter((key) => key !== "save_ns")) paths.push(`samples.${i}.values.${key}`);
  }
  const metrics = Object.fromEntries(paths.map((path) => [path, measurement(before, after, path, comparable)]));
  const costBefore = isCount(before?.process_lifetime_written_bytes) ? before.process_lifetime_written_bytes : null;
  const costAfter = isCount(after?.process_lifetime_written_bytes) ? after.process_lifetime_written_bytes : null;
  const reduction = metrics.whole_run_process_written_bytes.improvement_rate;
  return {
    comparable, reasons,
    cases: [{
      scenario: SCENARIOS.includes(before?.scenario) ? before.scenario : SCENARIOS.includes(after?.scenario) ? after.scenario : null,
      // Invalid shapes are not echoed; reports must not reproduce arbitrary bodies.
      before: beforeShape ? structuredClone(before) : null,
      after: afterShape ? structuredClone(after) : null,
      sample_counts: { before: Array.isArray(before?.samples) ? before.samples.length : null, after: Array.isArray(after?.samples) ? after.samples.length : null },
      metrics,
      performance: { process_write_reduction_target: 0.95, process_write_target_met: reduction === null ? null : reduction >= 0.95, enforced: false },
      experiment_cost: {
        before_process_lifetime_written_bytes: costBefore, after_process_lifetime_written_bytes: costAfter,
        pair_process_lifetime_written_bytes: costBefore !== null && costAfter !== null && Number.isSafeInteger(costBefore + costAfter) ? costBefore + costAfter : null,
        includes_discarded_runs: false,
      },
    }],
  };
}

const USAGE = "Usage: node scripts/compare-native-persistence.mjs --before <json> --after <json> --out <json>";
function main(args) {
  const options = {};
  for (let i = 0; i < args.length; i += 2) {
    const flag = args[i];
    if (!["--before", "--after", "--out"].includes(flag) || options[flag] || !args[i + 1] || args[i + 1].startsWith("--")) {
      process.stderr.write(`${USAGE}\n`);
      return 1;
    }
    options[flag] = args[i + 1];
  }
  if (Object.keys(options).length !== 3) {
    process.stderr.write(`${USAGE}\n`);
    return 1;
  }
  let before, after;
  try {
    // Refuse output aliases (including hard links) to keep raw inputs immutable.
    const inputs = [options["--before"], options["--after"]].map((path) => ({ path: realpathSync(path), stat: statSync(path) }));
    const output = resolve(options["--out"]);
    let outputStat = null;
    try { outputStat = statSync(output); } catch (error) { if (error.code !== "ENOENT") throw error; }
    if (inputs.some((input) => input.path === output || (outputStat && input.stat.dev === outputStat.dev && input.stat.ino === outputStat.ino))) {
      process.stderr.write("Comparison output must differ from raw inputs.\n");
      return 1;
    }
    before = JSON.parse(readFileSync(options["--before"], "utf8"));
    after = JSON.parse(readFileSync(options["--after"], "utf8"));
  } catch {
    process.stderr.write("Cannot read probe inputs as JSON.\n");
    return 1;
  }
  try {
    const result = compareRuns(before, after);
    writeFileSync(options["--out"], `${JSON.stringify(result, null, 2)}\n`);
    if (!result.comparable) process.stderr.write("Probe runs are incomplete or not comparable; see comparison reasons.\n");
    return result.comparable ? 0 : 1;
  } catch {
    process.stderr.write("Cannot generate or write comparison report.\n");
    return 1;
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) process.exitCode = main(process.argv.slice(2));
