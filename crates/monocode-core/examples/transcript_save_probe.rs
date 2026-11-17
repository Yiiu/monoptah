//! Measurement only: fixed synthetic inputs, a new temporary database, one JSON
//! result. No database-path option, provider calls, payload output, or WAL hooks.
use monocode_core::{
    store::{SaveConfiguration, SaveReason, SaveSample, Store, NATIVE_CHUNKS_MODE_VERSION},
    tool::ToolCall,
    transcript::Transcript,
};
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const MIB: usize = 1024 * 1024;
const BUDGET: u64 = 512 * MIB as u64;
const TIME_LIMIT: Duration = Duration::from_secs(120);
const FIXED_TIME: u64 = 1_700_000_000_000;
const SCENARIOS: [&str; 8] = [
    "append-1m",
    "append-10m",
    "append-40m",
    "same-value",
    "early-tool",
    "large-tool",
    "long-answer",
    "parallel",
];

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn json_hash(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| hash(&bytes))
        .map_err(|_| "fingerprint_failed".into())
}
fn ns(duration: Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

#[cfg(target_os = "macos")]
fn process_io() -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::rusage_info_v2>::zeroed();
    // SAFETY: the buffer matches RUSAGE_INFO_V2; read it only on success.
    let status = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as i32,
            libc::RUSAGE_INFO_V2,
            info.as_mut_ptr().cast(),
        )
    };
    (status == 0).then(|| unsafe { info.assume_init() }.ri_diskio_byteswritten)
}
#[cfg(not(target_os = "macos"))]
fn process_io() -> Option<u64> {
    None
}

fn io_delta(before: Option<u64>, after: Option<u64>) -> Option<u64> {
    after?.checked_sub(before?)
}
fn may_save(read: impl FnOnce() -> Option<u64>) -> bool {
    read().is_none_or(|written| written < BUDGET)
}

#[derive(Clone, Default, Serialize)]
struct Stage {
    attempted: bool,
    completed: bool,
    elapsed_ns: Option<u64>,
    io_before: Option<u64>,
    io_after: Option<u64>,
    process_written_bytes: Option<u64>,
    database_bytes: Option<u64>,
    wal_bytes: Option<u64>,
}
impl Stage {
    fn begin() -> (Self, Instant) {
        (
            Self {
                attempted: true,
                io_before: process_io(),
                ..Default::default()
            },
            Instant::now(),
        )
    }
    fn end(&mut self, start: Instant, path: &Path, completed: bool) {
        self.completed = completed;
        self.elapsed_ns = Some(ns(start.elapsed()));
        self.io_after = process_io();
        self.process_written_bytes = io_delta(self.io_before, self.io_after);
        self.database_bytes = file_size(path);
        self.wal_bytes = wal_size(path);
    }
}
#[derive(Clone, Default, Serialize)]
struct Stages {
    setup: Stage,
    /// Populated only when the workload converts a session to another storage
    /// backend; unattempted and entirely null otherwise.
    conversion: Stage,
    hot: Stage,
    final_save: Stage,
    checkpoint: Stage,
    close: Stage,
    restart: Stage,
}

/// Which storage backend produced this run. The default is the desktop blob
/// backend (no mode version); `--backend native-chunks` opts the sessions the
/// probe creates into the chunked backend, and the run reports the mode
/// version read back from the store's own marker.
#[derive(Clone, Serialize)]
struct Backend {
    kind: &'static str,
    mode_version: Option<u32>,
}

#[derive(Clone, Serialize)]
struct Event {
    index: usize,
    logical_ms: u64,
    session_index: usize,
    operation: &'static str,
    append_bytes: usize,
    append_sha256: String,
    reason: SaveReason,
}
#[derive(Clone, Serialize)]
struct Manifest {
    fixture_sha256: String,
    fixture_utf8_bytes: usize,
    completed_history_utf8_bytes_per_session: usize,
    active_payload_utf8_bytes_per_session: usize,
    initial_blocks_per_session: usize,
    initial_rows_per_session: usize,
    session_count: usize,
    fixture_version: u32,
    workload_sha256: String,
    events: Vec<Event>,
    save_parameters: Value,
    /// Supplemental-mode markers, present only when the run was invoked with
    /// `--supplemental-saves`: a fixed workload's manifest stays byte-identical.
    #[serde(skip_serializing_if = "Option::is_none")]
    supplemental: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supplemental_saves: Option<usize>,
}
#[derive(Clone, Serialize)]
struct SampleValues {
    reason: SaveReason,
    history_bytes_read: u64,
    serialized_bytes: u64,
    changed_rows: usize,
    commit_count: u64,
    lock_wait_ns: Option<u64>,
    lock_held_ns: Option<u64>,
    projection_ns: u64,
    encoding_ns: u64,
    commit_ns: u64,
    succeeded: bool,
    blocks_written: Option<u64>,
    chunks_written: Option<u64>,
    chunk_bytes_written: Option<u64>,
    chunks_written_missing_reason: Option<&'static str>,
    chunk_bytes_written_missing_reason: Option<&'static str>,
}
impl From<SaveSample> for SampleValues {
    fn from(sample: SaveSample) -> Self {
        Self {
            reason: sample.reason,
            history_bytes_read: sample.history_bytes_read,
            serialized_bytes: sample.serialized_bytes,
            changed_rows: sample.changed_rows,
            commit_count: sample.commit_count,
            lock_wait_ns: None,
            lock_held_ns: None,
            projection_ns: ns(sample.projection),
            encoding_ns: ns(sample.encoding),
            commit_ns: ns(sample.commit),
            succeeded: sample.succeeded,
            // The chunk counters' blob shape: null with the backend's explicit
            // missing reason, never a fabricated zero. A native-chunks run
            // swaps the sample's real counters in at the save_event swap point.
            blocks_written: None,
            chunks_written: None,
            chunk_bytes_written: None,
            chunks_written_missing_reason: Some("backend:blob"),
            chunk_bytes_written_missing_reason: Some("backend:blob"),
        }
    }
}
#[derive(Clone, Serialize)]
struct Sample {
    event_index: usize,
    session_index: usize,
    logical_ms: u64,
    started_ms: u64,
    completed_ms: u64,
    save_ns: u64,
    io_before: Option<u64>,
    io_after: Option<u64>,
    process_written_bytes: Option<u64>,
    values: Option<SampleValues>,
}
#[derive(Clone, Serialize, PartialEq, Eq)]
struct Fingerprint {
    blocks_sha256: String,
    rows_sha256: String,
    completed_history_sha256: String,
    unknown_metadata_sha256: String,
    receipts_sha256: String,
    queue_sha256: String,
    block_count: usize,
    row_count: usize,
}
#[derive(Clone, Default, Serialize)]
struct Integrity {
    before: Option<Fingerprint>,
    expected_rows_sha256: Option<String>,
    after_save: Option<Fingerprint>,
    after_restart: Option<Fingerprint>,
    initial_fixture_matches: Option<bool>,
    rows_match: Option<bool>,
    completed_history_match: Option<bool>,
    unknown_metadata_match: Option<bool>,
    receipts_match: Option<bool>,
    queue_match: Option<bool>,
    restart_match: Option<bool>,
    sqlite_ok: Option<bool>,
}
#[derive(Clone, Default, Serialize)]
struct Checkpoint {
    strategy: &'static str,
    busy: Option<i64>,
    log_frames: Option<i64>,
    checkpointed_frames: Option<i64>,
}
/// The supplemental-samples result. Present only when the run was invoked with
/// `--supplemental-saves`; the fixed 4-event workloads never grow a p95.
#[derive(Clone, Serialize)]
struct Supplemental {
    label: bool,
    samples: usize,
    p95_ns: Option<u64>,
    max_ns: Option<u64>,
    definition: &'static str,
}
/// One line, stating exactly which per-save duration the percentile ranks.
const SUPPLEMENTAL_DEFINITION: &str = "nearest-rank p95 (the ceil(0.95*n)-th smallest of the n sorted \
values) of samples[].save_ns, the wall-clock nanoseconds around the store write_rows call the report \
records per sample, over the supplemental routine small-append saves only; the final save is excluded";
#[derive(Clone, Serialize)]
struct Report {
    schema_version: u32,
    scenario: Option<String>,
    status: &'static str,
    incomplete_reasons: Vec<&'static str>,
    build: Value,
    sqlite: Option<SaveConfiguration>,
    backend: Backend,
    manifest: Option<Manifest>,
    completed_event_indexes: Vec<usize>,
    samples: Vec<Sample>,
    stages: Stages,
    checkpoint: Checkpoint,
    integrity: Integrity,
    whole_run_io_before: Option<u64>,
    whole_run_io_after: Option<u64>,
    whole_run_process_written_bytes: Option<u64>,
    process_lifetime_written_bytes: Option<u64>,
    io_budget_bytes: u64,
    budget_exhausted: bool,
    budget_overshoot_bytes: Option<u64>,
    wal_peak_bytes: Option<u64>,
    wal_peak_method: &'static str,
    wall_limit_ms: u64,
    elapsed_ns: u64,
    summary: Value,
    ui_response_ns: Option<u64>,
    ui_response_missing_reason: &'static str,
    /// The `--supplemental-saves` request, never serialized: the manifest
    /// labels and the `supplemental` object below carry it instead.
    #[serde(skip)]
    supplemental_saves: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supplemental: Option<Supplemental>,
}
impl Report {
    fn new(start_io: Option<u64>) -> Self {
        Self {
            schema_version: 2,
            scenario: None,
            status: "incomplete",
            incomplete_reasons: vec![],
            build: serde_json::from_str(include_str!(concat!(
                env!("OUT_DIR"),
                "/persistence_probe_build.json"
            )))
            .unwrap_or(Value::Null),
            sqlite: None,
            backend: Backend {
                kind: "blob",
                mode_version: None,
            },
            manifest: None,
            completed_event_indexes: vec![],
            samples: vec![],
            stages: Stages::default(),
            checkpoint: Checkpoint {
                strategy: "TRUNCATE",
                ..Default::default()
            },
            integrity: Integrity::default(),
            whole_run_io_before: start_io,
            whole_run_io_after: None,
            whole_run_process_written_bytes: None,
            process_lifetime_written_bytes: None,
            io_budget_bytes: BUDGET,
            budget_exhausted: false,
            budget_overshoot_bytes: None,
            wal_peak_bytes: None,
            wal_peak_method: "sampled_2ms_and_operation_boundaries",
            wall_limit_ms: TIME_LIMIT.as_millis() as u64,
            elapsed_ns: 0,
            summary: Value::Null,
            ui_response_ns: None,
            ui_response_missing_reason: "core_probe_has_no_ui",
            supplemental_saves: None,
            supplemental: None,
        }
    }
    fn fail(&mut self, reason: &'static str) {
        self.status = "incomplete";
        if !self.incomplete_reasons.contains(&reason) {
            self.incomplete_reasons.push(reason);
        }
    }
    fn finish(&mut self, start: Instant) {
        self.elapsed_ns = ns(start.elapsed());
        self.whole_run_io_after = process_io();
        self.process_lifetime_written_bytes = self.whole_run_io_after;
        self.whole_run_process_written_bytes =
            io_delta(self.whole_run_io_before, self.whole_run_io_after);
        self.budget_exhausted = self.whole_run_io_after.is_some_and(|bytes| bytes >= BUDGET);
        self.budget_overshoot_bytes = self
            .whole_run_io_after
            .map(|bytes| bytes.saturating_sub(BUDGET));
        if self.budget_exhausted {
            self.fail("io_budget_exhausted");
        }
        if self.whole_run_io_after.is_none() || self.whole_run_io_before.is_none() {
            if self.status == "skipped" {
                self.incomplete_reasons.push("process_io_unavailable");
            } else {
                self.fail("process_io_unavailable");
            }
        }
        if self.status == "complete"
            && (self.whole_run_process_written_bytes.is_none()
                || self.wal_peak_bytes.is_none()
                || self
                    .samples
                    .iter()
                    .any(|sample| sample.process_written_bytes.is_none())
                || self.samples.iter().any(|sample| {
                    sample
                        .values
                        .as_ref()
                        .is_some_and(|values| values.blocks_written.is_none())
                })
                || (self.stages.conversion.attempted
                    && self.stages.conversion.process_written_bytes.is_none())
                || [
                    &self.stages.setup,
                    &self.stages.hot,
                    &self.stages.final_save,
                    &self.stages.checkpoint,
                    &self.stages.close,
                    &self.stages.restart,
                ]
                .iter()
                .any(|stage| stage.process_written_bytes.is_none()))
        {
            self.fail("measurement_incomplete");
        }
        self.summary = summarize(&self.samples);
        if let Some(saves) = self.supplemental_saves {
            // Only the routine small-append saves feed the percentile: the
            // final save is a different reason and never a small-save sample.
            let mut durations = self
                .samples
                .iter()
                .filter(|sample| {
                    sample
                        .values
                        .as_ref()
                        .is_some_and(|values| matches!(values.reason, SaveReason::Routine))
                })
                .map(|sample| sample.save_ns)
                .collect::<Vec<_>>();
            durations.sort_unstable();
            self.supplemental = Some(Supplemental {
                label: true,
                samples: saves,
                p95_ns: p95_nearest_rank(&durations),
                max_ns: durations.last().copied(),
                definition: SUPPLEMENTAL_DEFINITION,
            });
            // The generic reason no longer applies: this run's p95 exists and
            // lives in the supplemental object above.
            if let Some(summary) = self.summary.as_object_mut() {
                summary.insert(
                    "p95_missing_reason".into(),
                    json!("p95_reported_in_supplemental_object"),
                );
            }
        }
    }
}

fn file_size(path: &Path) -> Option<u64> {
    match std::fs::metadata(path) {
        Ok(metadata) => Some(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(0),
        Err(_) => None,
    }
}
fn wal_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push("-wal");
    name.into()
}
fn wal_size(path: &Path) -> Option<u64> {
    file_size(&wal_path(path))
}
struct WalWatch {
    peak: Arc<AtomicU64>,
    missing: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl WalWatch {
    fn start(path: PathBuf) -> Self {
        let peak = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let missing = Arc::new(AtomicBool::new(false));
        let (sample_peak, sample_stop, sample_missing) =
            (peak.clone(), stop.clone(), missing.clone());
        let thread = std::thread::spawn(move || {
            while !sample_stop.load(Ordering::Relaxed) {
                if let Some(bytes) = wal_size(&path) {
                    sample_peak.fetch_max(bytes, Ordering::Relaxed);
                } else {
                    sample_missing.store(true, Ordering::Relaxed);
                }
                std::thread::park_timeout(Duration::from_millis(2));
            }
        });
        Self {
            peak,
            missing,
            stop,
            thread: Some(thread),
        }
    }
    fn observe(&self, path: &Path) {
        if let Some(bytes) = wal_size(path) {
            self.peak.fetch_max(bytes, Ordering::Relaxed);
        } else {
            self.missing.store(true, Ordering::Relaxed);
        }
    }
    fn peak(&self) -> Option<u64> {
        (!self.missing.load(Ordering::Relaxed)).then(|| self.peak.load(Ordering::Relaxed))
    }
}
impl Drop for WalWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

// The watchdog holds only a numeric snapshot, never a DB lock. On a stuck SQLite
// call it emits incomplete JSON and terminates this synthetic-only process.
fn publish(shared: &Mutex<Report>, report: &Report) {
    if let Ok(mut snapshot) = shared.lock() {
        *snapshot = report.clone();
    }
}
fn emit(report: &Report) {
    println!(
        "{}",
        serde_json::to_string(report).expect("numeric probe report serializes")
    );
}

fn fixture(history_bytes: usize, scenario: &str) -> Result<(Vec<Value>, usize), String> {
    let mut history = vec![
        json!({"id":"fixed-prompt","role":"user","text":"中文🙂 synthetic prompt","startedAt":FIXED_TIME,
            "appRequestId":"fixed-delivered","providerTurnId":"fixed-turn","durationMs":650,
            "turnModel":{"harness":"synthetic","id":"synthetic:fixed","name":"Synthetic"},
            "attachments":[{"id":"fixed-file","path":"synthetic/fixture.txt","kind":"file","mimeType":"text/plain","size":10,"futureAttachment":true}],"futureMetadata":{"fixed":[1,2,3]}}),
        json!({"id":"fixed-tool","role":"tool","text":"Synthetic tool","sentAt":FIXED_TIME,
            "tool":{"callId":"fixed-call","nativeName":"Read","nativeInput":"{}","kind":"read","title":"Synthetic tool","status":"running","detail":"synthetic output","futureToolField":[1,2]},"futureMetadata":true}),
        json!({"id":"fixed-interjection","role":"system","text":"Synthetic notice","sentAt":FIXED_TIME,
            "interjection":{"customType":"custom","severity":"info"},"futureMetadata":7}),
        json!({"id":"fixed-card","role":"assistant","text":"","monoCard":{"type":"document","documentId":"fixed-document","title":"Synthetic card","futureCard":true},"futureMetadata":9}),
        json!({"id":"fixed-history","role":"assistant","text":"","sentAt":FIXED_TIME,"futureMetadata":{"complete":true}}),
    ];
    history[1]["tool"]["status"] = json!(if scenario == "early-tool" {
        "running"
    } else {
        "completed"
    });
    let overhead = serde_json::to_vec(&history)
        .map_err(|_| "fixture_encoding_failed")?
        .len();
    let padding = history_bytes
        .checked_sub(overhead)
        .ok_or("fixture_too_small")?;
    history[4]["text"] = json!("h".repeat(padding));
    assert_eq!(serde_json::to_vec(&history).unwrap().len(), history_bytes);
    let active_bytes = if matches!(scenario, "large-tool" | "long-answer") {
        5 * MIB
    } else {
        0
    };
    if scenario == "large-tool" {
        history.push(json!({"id":"fixed-active","role":"tool","text":"Synthetic active tool","sentAt":FIXED_TIME,
            "tool":{"callId":"fixed-active-call","nativeName":"Read","nativeInput":"x".repeat(active_bytes),"kind":"read","title":"Synthetic active tool","status":"running","detail":"synthetic output","futureToolField":true},"futureMetadata":{"active":true}}));
    } else {
        history.push(json!({"id":"fixed-active","role":"assistant","text":"a".repeat(active_bytes),"sentAt":FIXED_TIME,"futureMetadata":{"active":true}}));
    }
    Ok((history, active_bytes))
}
fn queue() -> Value {
    json!([{"id":"fixed-pending","text":"Synthetic pending","futureQueue":{"keep":true}}])
}
fn events(scenario: &str, sessions: usize) -> Vec<Event> {
    let operation = match scenario {
        "same-value" => "same_value",
        "early-tool" => "complete_early_tool",
        "large-tool" => "append_tool_input",
        _ => "append_answer",
    };
    let count = if scenario == "early-tool" { 1 } else { 4 };
    let append = if matches!(scenario, "same-value" | "early-tool") {
        "".into()
    } else {
        "x".repeat(1024)
    };
    let mut events = vec![];
    for tick in 0..count {
        for session_index in 0..sessions {
            events.push(Event {
                index: events.len(),
                logical_ms: tick * 650,
                session_index,
                operation,
                append_bytes: append.len(),
                append_sha256: hash(append.as_bytes()),
                reason: SaveReason::Routine,
            });
        }
    }
    for session_index in 0..sessions {
        events.push(Event {
            index: events.len(),
            logical_ms: 2600,
            session_index,
            operation: "final_save",
            append_bytes: 0,
            append_sha256: hash(b""),
            reason: SaveReason::Final,
        });
    }
    events
}
/// The supplemental workload: `saves` identical small-append events (the
/// append-1m hot shape) on the same 650 ms logical cadence — event i at
/// i·650 ms — finishing with one final save per session at `saves`·650 ms. It
/// replaces the scenario's fixed 4-event plan, so a supplemental run is never
/// pair material with a fixed one.
fn supplemental_events(saves: usize, sessions: usize) -> Vec<Event> {
    let append = "x".repeat(1024);
    let mut events = vec![];
    for tick in 0..saves {
        for session_index in 0..sessions {
            events.push(Event {
                index: events.len(),
                logical_ms: tick as u64 * 650,
                session_index,
                operation: "append_answer",
                append_bytes: append.len(),
                append_sha256: hash(append.as_bytes()),
                reason: SaveReason::Routine,
            });
        }
    }
    for session_index in 0..sessions {
        events.push(Event {
            index: events.len(),
            logical_ms: saves as u64 * 650,
            session_index,
            operation: "final_save",
            append_bytes: 0,
            append_sha256: hash(b""),
            reason: SaveReason::Final,
        });
    }
    events
}
/// The save parameters both workloads declare. A fixed run always passes 2600;
/// a supplemental run passes `saves`·650 — the rest of the object is identical
/// either way, so a fixed run's manifest bytes do not move.
fn save_parameters(final_logical_ms: u64, sessions: usize) -> Value {
    json!({"routine_interval_ms":650,"final_logical_ms":final_logical_ms,"append_bytes":1024,
        "final_save":"last_message_suffix_even_if_unchanged","database":"new_temporary_blob_schema","parallel_sessions":sessions,
        "parallel_policy":"barrier_per_logical_tick_shared_store_mutex","busy_timeout_ms":5000})
}
/// A run is complete only when every planned event was sampled, in order, with
/// no extra sample — the same rule for a supplemental plan, whose event count
/// simply follows its own `saves` value.
fn workload_fully_sampled(
    completed_event_indexes: &[usize],
    sample_count: usize,
    planned: &[Event],
) -> bool {
    completed_event_indexes
        == planned
            .iter()
            .map(|event| event.index)
            .collect::<Vec<_>>()
            .as_slice()
        && sample_count == planned.len()
}

fn unknown(value: &Value, path: &str, found: &mut Vec<Value>) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                // This derived display envelope may duplicate original unknown fields.
                // Fingerprint the source fields rather than counting a new alias.
                if key == "nativePresentation" {
                    continue;
                }
                let path = format!("{path}/{key}");
                if key.starts_with("future") {
                    found.push(json!([path, value]));
                }
                unknown(value, &path, found);
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                unknown(value, &format!("{path}/{index}"), found);
            }
        }
        _ => {}
    }
}
fn fingerprint(conn: &Connection, store: &Store, ids: &[i64]) -> Result<Fingerprint, String> {
    let mut all_blocks = vec![];
    let mut all_rows = vec![];
    let mut histories = vec![];
    let mut metadata = vec![];
    let mut receipts = vec![];
    let mut queues = vec![];
    for (index, id) in ids.iter().enumerate() {
        let queue: String = conn
            .query_row(
                "SELECT queued_messages_json FROM sessions WHERE id=?1",
                [format!("fixed-session-{index}")],
                |row| row.get(0),
            )
            .map_err(|_| "integrity_read_failed")?;
        // Blocks come through the store's active backend: a chunked session's
        // blocks live in the native tables while its `blocks_json` stays the
        // untouched fixture, so reading the column would compare the fixture
        // with itself instead of what the saves produced.
        let blocks: Vec<Value> = store
            .stored_blocks(*id)
            .map_err(|_| "integrity_read_failed")?;
        let queue: Value = serde_json::from_str(&queue).map_err(|_| "integrity_decode_failed")?;
        unknown(
            &json!({"blocks":blocks,"queue":queue}),
            &index.to_string(),
            &mut metadata,
        );
        histories.push(
            blocks
                .iter()
                .take(5)
                .enumerate()
                .filter(|(index, _)| *index != 1)
                .map(|(_, block)| block.clone())
                .collect::<Vec<_>>(),
        );
        receipts.push(blocks.iter().map(|block| json!({"id":block["id"],"appRequestId":block["appRequestId"],
            "providerTurnId":block["providerTurnId"],"turnModel":block["turnModel"],"durationMs":block["durationMs"]})).collect::<Vec<_>>());
        all_rows.push(
            store
                .load(*id)
                .map_err(|_| "integrity_projection_failed")?
                .rows,
        );
        all_blocks.push(blocks);
        queues.push(queue);
    }
    Ok(Fingerprint {
        blocks_sha256: json_hash(&all_blocks)?,
        rows_sha256: json_hash(&all_rows)?,
        completed_history_sha256: json_hash(&histories)?,
        unknown_metadata_sha256: json_hash(&metadata)?,
        receipts_sha256: json_hash(&receipts)?,
        queue_sha256: json_hash(&queues)?,
        block_count: all_blocks.iter().map(Vec::len).sum(),
        row_count: all_rows.iter().map(Vec::len).sum(),
    })
}
fn mutate(transcript: &mut Transcript, event: &Event) -> Result<usize, &'static str> {
    match event.operation {
        "append_answer" => Ok(transcript.append(&"x".repeat(event.append_bytes))),
        "append_tool_input" | "complete_early_tool" => {
            let index = if event.operation == "complete_early_tool" {
                transcript.rows.iter().position(|row| row.role == "tool")
            } else {
                transcript
                    .rows
                    .iter()
                    .rposition(|row| row.first && row.role == "tool")
            }
            .ok_or("fixture_tool_missing")?;
            // Tool rows are a single serialized ToolCall, even for a large input.
            let mut call: ToolCall = serde_json::from_str(&transcript.rows[index].text)
                .map_err(|_| "fixture_tool_invalid")?;
            if event.operation == "complete_early_tool" {
                call.state = monocode_core::tool::ToolState::Completed;
            } else {
                call.input.push_str(&"x".repeat(event.append_bytes));
            }
            transcript.rows[index].text =
                serde_json::to_string(&call).map_err(|_| "fixture_tool_invalid")?;
            Ok(index)
        }
        "same_value" | "final_save" => Ok(transcript
            .rows
            .iter()
            .rposition(|row| row.first)
            .unwrap_or(0)),
        _ => Err("invalid_operation"),
    }
}
fn save_event(
    store: &Mutex<Store>,
    transcript: &mut Transcript,
    id: i64,
    event: &Event,
    start: Instant,
    blocks: usize,
    native_chunks: bool,
) -> Result<Sample, &'static str> {
    let from = mutate(transcript, event)?;
    let mut store = store.lock().map_err(|_| "store_lock_failed")?;
    // All concurrent sessions share this budget gate and store lock.
    let io_before = process_io();
    if !may_save(|| io_before) {
        return Err("io_budget_exhausted");
    }
    let started_ms = start.elapsed().as_millis() as u64;
    let save_start = Instant::now();
    let result = store.write_rows(id, from, &transcript.rows[from..]);
    let save_ns = ns(save_start.elapsed());
    let taken = store.take_save_sample();
    let counted = taken.as_ref().map(|sample| {
        (
            sample.blocks_written,
            sample.chunks_written,
            sample.chunk_bytes_written,
        )
    });
    let mut values = taken.map(SampleValues::from);
    if let Some(values) = &mut values {
        values.reason = event.reason;
        values.succeeded &= result.is_ok();
        match (counted, native_chunks) {
            // The chunked backend's writer counted what it actually wrote,
            // chunk by chunk; its samples carry those numbers verbatim.
            (Some((blocks_written, chunks_written, chunk_bytes_written)), true) => {
                values.blocks_written = Some(blocks_written);
                values.chunks_written = Some(chunks_written);
                values.chunk_bytes_written = Some(chunk_bytes_written);
                values.chunks_written_missing_reason = None;
                values.chunk_bytes_written_missing_reason = None;
            }
            // The blob backend rewrites the session's whole block array per
            // save; no workload operation adds or removes a block, so this
            // width is constant and the probe supplies it.
            _ => values.blocks_written = Some(blocks as u64),
        }
    }
    let io_after = process_io();
    Ok(Sample {
        event_index: event.index,
        session_index: event.session_index,
        logical_ms: event.logical_ms,
        started_ms,
        completed_ms: start.elapsed().as_millis() as u64,
        save_ns,
        io_before,
        io_after,
        process_written_bytes: io_delta(io_before, io_after),
        values,
    })
}
/// Nearest-rank p95 over already-sorted durations: the ceil(0.95·n)-th
/// smallest (1-based) value, in integer arithmetic — no interpolation, no
/// external crate. `None` when there is nothing to rank.
fn p95_nearest_rank(sorted: &[u64]) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (95 * sorted.len()).div_ceil(100);
    sorted.get(rank.clamp(1, sorted.len()) - 1).copied()
}
fn summarize(samples: &[Sample]) -> Value {
    if samples.is_empty() {
        return json!({"sample_count":0,"successful_sample_count":0,"totals":null,"max":null,
            "p95_ns":null,"p95_missing_reason":"single_fixed_workload_fewer_than_100_samples"});
    }
    let values = samples
        .iter()
        .filter_map(|sample| sample.values.as_ref())
        .collect::<Vec<_>>();
    // The new counters aggregate only known values: an all-null backend must
    // report null totals, never a fabricated zero.
    let known = |field: fn(&SampleValues) -> Option<u64>| {
        values
            .iter()
            .filter_map(|value| field(value))
            .collect::<Vec<u64>>()
    };
    let blocks_written = known(|value| value.blocks_written);
    let chunks_written = known(|value| value.chunks_written);
    let chunk_bytes_written = known(|value| value.chunk_bytes_written);
    let total = |known: &[u64]| (!known.is_empty()).then(|| known.iter().sum::<u64>());
    json!({"sample_count":samples.len(), "successful_sample_count":values.iter().filter(|value| value.succeeded).count(),
        "totals":{"history_bytes_read":values.iter().map(|v|v.history_bytes_read).sum::<u64>(),
            "serialized_bytes":values.iter().map(|v|v.serialized_bytes).sum::<u64>(),
            "changed_rows":values.iter().map(|v|v.changed_rows).sum::<usize>(),
            "commit_count":values.iter().map(|v|v.commit_count).sum::<u64>(),
            "projection_ns":values.iter().map(|v|v.projection_ns).sum::<u64>(),
            "encoding_ns":values.iter().map(|v|v.encoding_ns).sum::<u64>(),
            "commit_ns":values.iter().map(|v|v.commit_ns).sum::<u64>(),
            "save_ns":samples.iter().map(|v|v.save_ns).sum::<u64>(),"lock_wait_ns":null,"lock_held_ns":null,
            "blocks_written":total(&blocks_written),"chunks_written":total(&chunks_written),
            "chunk_bytes_written":total(&chunk_bytes_written)},
        "max":{"history_bytes_read":values.iter().map(|v|v.history_bytes_read).max(),
            "serialized_bytes":values.iter().map(|v|v.serialized_bytes).max(),
            "changed_rows":values.iter().map(|v|v.changed_rows).max(),
            "commit_count":values.iter().map(|v|v.commit_count).max(),
            "projection_ns":values.iter().map(|v|v.projection_ns).max(),
            "encoding_ns":values.iter().map(|v|v.encoding_ns).max(),"commit_ns":values.iter().map(|v|v.commit_ns).max(),
            "save_ns":samples.iter().map(|v|v.save_ns).max(),"lock_wait_ns":null,"lock_held_ns":null,
            "blocks_written":blocks_written.iter().copied().max(),
            "chunks_written":chunks_written.iter().copied().max(),
            "chunk_bytes_written":chunk_bytes_written.iter().copied().max()},
        "p95_ns":null,"p95_missing_reason":"single_fixed_workload_fewer_than_100_samples"})
}
fn record_sample(
    report: &mut Report,
    result: Result<Sample, &'static str>,
) -> Result<(), &'static str> {
    let sample = result?;
    let succeeded = sample
        .values
        .as_ref()
        .is_some_and(|values| values.succeeded);
    if succeeded {
        report.completed_event_indexes.push(sample.event_index);
    }
    report.samples.push(sample);
    if succeeded {
        Ok(())
    } else {
        Err("save_failed_or_sample_missing")
    }
}

fn run(
    report: &mut Report,
    shared: &Mutex<Report>,
    native_chunks: bool,
) -> Result<(), &'static str> {
    let scenario = report.scenario.clone().ok_or("invalid_cli")?;
    if report.build["complete"] != true || report.build["profile"] != "native-dev" {
        return Err("build_identity_incomplete_or_wrong_profile");
    }
    if scenario == "append-40m" && process_io().is_none() {
        report.status = "skipped";
        report
            .incomplete_reasons
            .push("large_scenario_requires_process_io");
        return Ok(());
    }
    let (stage, setup_start) = Stage::begin();
    report.stages.setup = stage;
    publish(shared, report);
    let history_bytes = if report.supplemental_saves.is_some() {
        // A supplemental run measures small saves on the append-1m fixture,
        // whatever scenario name the invocation carried.
        MIB
    } else {
        match scenario.as_str() {
            "append-10m" => 10 * MIB,
            "append-40m" => 40 * MIB,
            _ => MIB,
        }
    };
    let sessions = if scenario == "parallel" { 4 } else { 1 };
    let fixture_scenario = if report.supplemental_saves.is_some() {
        "append-1m"
    } else {
        scenario.as_str()
    };
    let (blocks, active_bytes) =
        fixture(history_bytes, fixture_scenario).map_err(|_| "fixture_failed")?;
    let fixtures = vec![blocks; sessions];
    // The blob backend rewrites each session's whole block array per save; no
    // workload operation adds or removes a block, so this width is constant.
    let blocks_per_session: Vec<usize> = fixtures.iter().map(Vec::len).collect();
    let fixture_raw = serde_json::to_vec(&fixtures).map_err(|_| "fixture_failed")?;
    let planned = match report.supplemental_saves {
        Some(saves) => supplemental_events(saves, sessions),
        None => events(&scenario, sessions),
    };
    let temp = tempfile::tempdir().map_err(|_| "temporary_directory_failed")?;
    let path = temp.path().join("synthetic.db");
    let watch = WalWatch::start(path.clone());
    // Bootstrap only this newly created temporary file; never accept a DB path.
    if !may_save(process_io) {
        return Err("io_budget_exhausted");
    }
    let bootstrap = Store::open_monocode(&path).map_err(|_| "temporary_database_failed")?;
    report.sqlite = Some(
        bootstrap
            .save_configuration()
            .map_err(|_| "sqlite_configuration_failed")?,
    );
    drop(bootstrap);
    let mut conn = Connection::open(&path).map_err(|_| "temporary_database_failed")?;
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|_| "temporary_database_failed")?;
    conn.execute_batch("PRAGMA foreign_keys=ON")
        .map_err(|_| "temporary_database_failed")?;
    if !may_save(process_io) {
        return Err("io_budget_exhausted");
    }
    let tx = conn.transaction().map_err(|_| "fixture_setup_failed")?;
    for (index, blocks) in fixtures.iter().enumerate() {
        tx.execute("INSERT INTO sessions(id,cwd,harness,model,runtime_mode,title,blocks_json,created_at,updated_at,has_user_message,queued_messages_json,surface)
            VALUES(?1,'synthetic','synthetic','synthetic:fixed','supervised','Synthetic',?2,?3,?3,1,?4,'app')",
            params![format!("fixed-session-{index}"), serde_json::to_string(blocks).map_err(|_| "fixture_setup_failed")?, FIXED_TIME.min(i64::MAX as u64) as i64, queue().to_string()])
            .map_err(|_| "fixture_setup_failed")?;
    }
    // The backend flag opts the sessions this probe creates into the chunked
    // backend: one mode marker per session, the same marker the store's save
    // path dispatches on. Nothing else moves — the fixture, manifest, workload
    // and save parameters stay byte-identical, so Before/After pairing holds.
    // (A supplemental run replaces the workload by design and labels its
    // manifest; it is never pair material.)
    if native_chunks {
        for index in 0..sessions {
            tx.execute(
                "INSERT INTO native_sessions(session_id,mode_version,session_revision) VALUES(?1,?2,0)",
                params![
                    format!("fixed-session-{index}"),
                    i64::from(NATIVE_CHUNKS_MODE_VERSION)
                ],
            )
            .map_err(|_| "fixture_setup_failed")?;
        }
    }
    tx.commit().map_err(|_| "fixture_setup_failed")?;
    watch.observe(&path);
    let store = Store::open_monocode(&path).map_err(|_| "temporary_database_failed")?;
    if Some(
        store
            .save_configuration()
            .map_err(|_| "sqlite_configuration_failed")?,
    ) != report.sqlite
    {
        return Err("sqlite_configuration_changed");
    }
    let summaries = store.sessions().map_err(|_| "fixture_load_failed")?;
    let mut ids = vec![];
    for index in 0..sessions {
        let original = format!("fixed-session-{index}");
        ids.push(
            summaries
                .iter()
                .find(|session| {
                    store.persisted_session_id(session.id).ok().as_deref()
                        == Some(original.as_str())
                })
                .ok_or("fixture_load_failed")?
                .id,
        );
    }
    if native_chunks {
        // The opt-in itself: store every session's fixture blocks through the
        // chunked backend's own full-replace path (the marker is already in
        // place), so the native copy carries the fixture's exact ids,
        // receipts and unknown metadata — not rows re-derived blocks — and
        // every workload save exercises the sparse writer against them. The
        // desktop `blocks_json` keeps the fixture bytes exactly as inserted;
        // the chunked backend owns the transcript from here on, and the
        // restart-integrity fingerprint reads it through the store's native
        // reader. The reported mode version is read back from the store,
        // never echoed from the flag. The transcripts load afterwards:
        // with the marker in place a load reads the native tables, which
        // are empty until this store lands.
        for (id, fixture) in ids.iter().zip(&fixtures) {
            store
                .store_blocks(*id, fixture)
                .map_err(|_| "backend_opt_in_failed")?;
        }
        let mode_version = store
            .storage_mode_version(ids[0])
            .map_err(|_| "backend_mode_read_failed")?;
        report.backend = Backend {
            kind: "native_chunks",
            mode_version,
        };
        if mode_version.is_none() {
            return Err("backend_opt_in_failed");
        }
    }
    let mut transcripts = ids
        .iter()
        .map(|id| store.load(*id).map_err(|_| "fixture_load_failed"))
        .collect::<Result<Vec<_>, _>>()?;
    let final_logical_ms = report
        .supplemental_saves
        .map_or(2600, |saves| saves as u64 * 650);
    let parameters = save_parameters(final_logical_ms, sessions);
    report.manifest = Some(Manifest {
        fixture_sha256: hash(&fixture_raw),
        fixture_utf8_bytes: fixture_raw.len(),
        completed_history_utf8_bytes_per_session: history_bytes,
        active_payload_utf8_bytes_per_session: active_bytes,
        initial_blocks_per_session: fixtures[0].len(),
        initial_rows_per_session: transcripts[0].rows.len(),
        session_count: sessions,
        fixture_version: 1,
        workload_sha256: json_hash(&json!({"events":planned,"save_parameters":parameters}))
            .map_err(|_| "workload_hash_failed")?,
        events: planned.clone(),
        save_parameters: parameters,
        supplemental: report.supplemental_saves.map(|_| true),
        supplemental_saves: report.supplemental_saves,
    });
    let before = fingerprint(&conn, &store, &ids).map_err(|_| "initial_integrity_failed")?;
    report.integrity.initial_fixture_matches =
        Some(before.blocks_sha256 == report.manifest.as_ref().unwrap().fixture_sha256);
    report.integrity.before = Some(before);
    report.stages.setup.end(setup_start, &path, true);
    watch.observe(&path);
    report.wal_peak_bytes = watch.peak();
    publish(shared, report);
    if report.integrity.initial_fixture_matches != Some(true) {
        return Err("initial_fixture_mismatch");
    }
    let store = Mutex::new(store);
    let workload_start = Instant::now();
    let (stage, hot_start) = Stage::begin();
    report.stages.hot = stage;
    let mut hot_error = None;
    let ticks: Vec<u64> = match report.supplemental_saves {
        Some(saves) => (0..saves).map(|tick| tick as u64 * 650).collect(),
        None => vec![0, 650, 1300, 1950],
    };
    for tick in ticks {
        let wave = planned
            .iter()
            .filter(|event| event.logical_ms == tick && event.operation != "final_save")
            .collect::<Vec<_>>();
        if wave.is_empty() {
            continue;
        }
        if let Some(delay) = Duration::from_millis(tick).checked_sub(workload_start.elapsed()) {
            std::thread::sleep(delay);
        }
        if scenario == "parallel" {
            let barrier = std::sync::Barrier::new(sessions);
            let results = std::thread::scope(|scope| {
                let handles = transcripts
                    .iter_mut()
                    .zip(&ids)
                    .zip(&wave)
                    .map(|((transcript, id), event)| {
                        let (store, barrier) = (&store, &barrier);
                        let blocks = &blocks_per_session;
                        scope.spawn(move || {
                            barrier.wait();
                            save_event(
                                store,
                                transcript,
                                *id,
                                event,
                                workload_start,
                                blocks[event.session_index],
                                native_chunks,
                            )
                        })
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|handle| handle.join().unwrap_or(Err("parallel_worker_panicked")))
                    .collect::<Vec<_>>()
            });
            for result in results {
                if let Err(error) = record_sample(report, result) {
                    hot_error = Some(error);
                }
            }
        } else {
            let event = wave[0];
            if let Err(error) = record_sample(
                report,
                save_event(
                    &store,
                    &mut transcripts[0],
                    ids[0],
                    event,
                    workload_start,
                    blocks_per_session[0],
                    native_chunks,
                ),
            ) {
                hot_error = Some(error);
            }
        }
        watch.observe(&path);
        report.wal_peak_bytes = watch.peak();
        publish(shared, report);
        if hot_error.is_some() {
            break;
        }
    }
    report.stages.hot.end(hot_start, &path, hot_error.is_none());
    if let Some(error) = hot_error {
        return Err(error);
    }
    let (stage, final_start) = Stage::begin();
    report.stages.final_save = stage;
    if let Some(delay) =
        Duration::from_millis(final_logical_ms).checked_sub(workload_start.elapsed())
    {
        std::thread::sleep(delay);
    }
    let mut final_error = None;
    for event in planned
        .iter()
        .filter(|event| event.operation == "final_save")
    {
        if let Err(error) = record_sample(
            report,
            save_event(
                &store,
                &mut transcripts[event.session_index],
                ids[event.session_index],
                event,
                workload_start,
                blocks_per_session[event.session_index],
                native_chunks,
            ),
        ) {
            final_error = Some(error);
            break;
        }
        watch.observe(&path);
    }
    report
        .stages
        .final_save
        .end(final_start, &path, final_error.is_none());
    report.wal_peak_bytes = watch.peak();
    publish(shared, report);
    if let Some(error) = final_error {
        return Err(error);
    }
    if !workload_fully_sampled(
        &report.completed_event_indexes,
        report.samples.len(),
        &planned,
    ) {
        return Err("event_sequence_incomplete");
    }
    let expected_rows = transcripts
        .iter()
        .map(|transcript| &transcript.rows)
        .collect::<Vec<_>>();
    report.integrity.expected_rows_sha256 =
        Some(json_hash(&expected_rows).map_err(|_| "integrity_hash_failed")?);
    let store = store.into_inner().map_err(|_| "store_lock_failed")?;
    let after = fingerprint(&conn, &store, &ids).map_err(|_| "saved_integrity_failed")?;
    // The per-sample blocks_written width claims the blob backend rewrote
    // every block; the stored fingerprint must agree with that width.
    if after.block_count != blocks_per_session.iter().sum::<usize>() {
        return Err("block_count_mismatch");
    }
    let before = report.integrity.before.as_ref().unwrap();
    report.integrity.rows_match =
        Some(Some(&after.rows_sha256) == report.integrity.expected_rows_sha256.as_ref());
    report.integrity.completed_history_match =
        Some(after.completed_history_sha256 == before.completed_history_sha256);
    report.integrity.unknown_metadata_match =
        Some(after.unknown_metadata_sha256 == before.unknown_metadata_sha256);
    report.integrity.receipts_match = Some(after.receipts_sha256 == before.receipts_sha256);
    report.integrity.queue_match = Some(after.queue_sha256 == before.queue_sha256);
    report.integrity.after_save = Some(after);
    let (stage, checkpoint_start) = Stage::begin();
    report.stages.checkpoint = stage;
    watch.observe(&path);
    report.wal_peak_bytes = watch.peak();
    publish(shared, report);
    // One identical, explicit final checkpoint. No checkpoint on sample reads.
    let checkpoint = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    });
    let checkpoint_ok = if let Ok((busy, log, done)) = checkpoint {
        report.checkpoint.busy = Some(busy);
        report.checkpoint.log_frames = Some(log);
        report.checkpoint.checkpointed_frames = Some(done);
        busy == 0 && log == done
    } else {
        false
    };
    report
        .stages
        .checkpoint
        .end(checkpoint_start, &path, checkpoint_ok);
    let (stage, close_start) = Stage::begin();
    report.stages.close = stage;
    drop(store);
    drop(conn);
    report.stages.close.end(close_start, &path, true);
    publish(shared, report);
    let (stage, restart_start) = Stage::begin();
    report.stages.restart = stage;
    let store = Store::open_monocode(&path).map_err(|_| "restart_failed")?;
    let conn = Connection::open(&path).map_err(|_| "restart_failed")?;
    let restarted = fingerprint(&conn, &store, &ids).map_err(|_| "restart_integrity_failed")?;
    report.integrity.restart_match = Some(report.integrity.after_save.as_ref() == Some(&restarted));
    report.integrity.after_restart = Some(restarted);
    report.integrity.sqlite_ok = Some(
        conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .is_ok_and(|result| result == "ok"),
    );
    drop(store);
    drop(conn);
    report.stages.restart.end(restart_start, &path, true);
    watch.observe(&path);
    report.wal_peak_bytes = watch.peak();
    if !checkpoint_ok {
        return Err("checkpoint_failed_or_busy");
    }
    if [
        report.integrity.rows_match,
        report.integrity.completed_history_match,
        report.integrity.unknown_metadata_match,
        report.integrity.receipts_match,
        report.integrity.queue_match,
        report.integrity.restart_match,
        report.integrity.sqlite_ok,
    ]
    .iter()
    .any(|value| *value != Some(true))
    {
        return Err("integrity_mismatch");
    }
    report.status = "complete";
    Ok(())
}

/// The parsed probe CLI. Equality against a `(&scenario, backend)` tuple is
/// the (scenario, backend) projection — the view the assertions that predate
/// the supplemental flag assert — while `Cli == Cli` compares every field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cli {
    scenario: &'static str,
    native_chunks: bool,
    supplemental_saves: Option<usize>,
}
impl PartialEq<(&'static str, bool)> for Cli {
    fn eq(&self, (scenario, native_chunks): &(&'static str, bool)) -> bool {
        (self.scenario, self.native_chunks) == (*scenario, *native_chunks)
    }
}

/// Parse the probe's arguments: `--scenario <name>` (required, one of
/// [`SCENARIOS`]), the optional `--backend native-chunks` and the optional
/// `--supplemental-saves <N>` (a positive count) in any order. The backend
/// flag is invocation-level only — it never reaches the manifest, fixture,
/// workload or save parameters, which must stay byte-identical for
/// Before/After pairing. The supplemental flag is the labeled exception: it
/// replaces the fixed workload with N small-append saves and marks the
/// manifest, so a supplemental run is never pair material.
fn parse_scenario(args: &[String]) -> Result<Cli, &'static str> {
    let mut scenario = None;
    let mut native_chunks = false;
    let mut supplemental_saves = None;
    let mut flags = args.iter();
    while let Some(flag) = flags.next() {
        match flag.as_str() {
            "--scenario" => {
                let Some(value) = flags.next() else {
                    return Err("invalid_cli");
                };
                if scenario.is_some() {
                    return Err("invalid_cli");
                }
                scenario = SCENARIOS
                    .iter()
                    .find(|known| **known == value.as_str())
                    .copied();
                if scenario.is_none() {
                    return Err("invalid_cli");
                }
            }
            "--backend" => {
                let Some(value) = flags.next() else {
                    return Err("invalid_cli");
                };
                // The blob backend needs no flag: it is the default this probe
                // measures, so only the chunked opt-in is spelled out.
                if native_chunks || value != "native-chunks" {
                    return Err("invalid_cli");
                }
                native_chunks = true;
            }
            "--supplemental-saves" => {
                let Some(value) = flags.next() else {
                    return Err("invalid_cli");
                };
                if supplemental_saves.is_some() {
                    return Err("invalid_cli");
                }
                // Zero saves is no workload; anything non-numeric or negative
                // (which usize rejects) is not a count.
                supplemental_saves = Some(
                    value
                        .parse::<usize>()
                        .ok()
                        .filter(|saves| *saves > 0)
                        .ok_or("invalid_cli")?,
                );
            }
            _ => return Err("invalid_cli"),
        }
    }
    scenario
        .map(|scenario| Cli {
            scenario,
            native_chunks,
            supplemental_saves,
        })
        .ok_or("invalid_cli")
}
fn main() {
    let start = Instant::now();
    let mut report = Report::new(process_io());
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let cli = match parse_scenario(&args) {
        Ok(cli) => {
            report.scenario = Some(cli.scenario.into());
            report.supplemental_saves = cli.supplemental_saves;
            cli
        }
        Err(error) => {
            report.fail(error);
            report.finish(start);
            emit(&report);
            std::process::exit(2);
        }
    };
    let shared = Arc::new(Mutex::new(report.clone()));
    let watchdog_report = shared.clone();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if done_rx.recv_timeout(TIME_LIMIT).is_err() {
            let mut snapshot = watchdog_report.lock().unwrap().clone();
            snapshot.fail("wall_time_limit");
            snapshot.finish(start);
            emit(&snapshot);
            std::process::exit(2);
        }
    });
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(&mut report, &shared, cli.native_chunks)
    })) {
        Ok(Ok(())) => {}
        Ok(Err(reason)) => report.fail(reason),
        Err(_) => report.fail("probe_panicked"),
    }
    report.finish(start);
    let _ = done_tx.send(());
    let _ = watchdog.join();
    emit(&report);
    if report.status != "complete" {
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_stops_before_next_save() {
        let readings = std::cell::Cell::new(BUDGET - 1);
        let saves = std::cell::Cell::new(0);
        for _ in 0..4 {
            if !may_save(|| Some(readings.get())) {
                break;
            }
            saves.set(saves.get() + 1);
            readings.set(readings.get() + 4096);
        }
        assert_eq!(saves.get(), 1);
        assert_eq!(readings.get() - BUDGET, 4095);
        assert!(!may_save(|| Some(BUDGET)));
        assert!(may_save(|| None));
    }

    #[test]
    fn missing_io_is_null() {
        assert_eq!(io_delta(None, Some(7)), None);
        assert_eq!(io_delta(Some(7), None), None);
        assert_eq!(io_delta(Some(8), Some(7)), None);
        let report = Report::new(None);
        let json = serde_json::to_value(report).unwrap();
        assert_eq!(json["schema_version"], 2);
        assert!(json.get("version").is_none());
        assert_eq!(json["backend"]["kind"], "blob");
        assert!(json["backend"]["mode_version"].is_null());
        assert_eq!(json["stages"]["conversion"]["attempted"], false);
        assert!(json["stages"]["conversion"]["elapsed_ns"].is_null());
        assert!(json["whole_run_process_written_bytes"].is_null());
        assert!(json["process_lifetime_written_bytes"].is_null());
        let sample = serde_json::to_value(SampleValues::from(SaveSample::default())).unwrap();
        assert!(sample["lock_wait_ns"].is_null() && sample["lock_held_ns"].is_null());
    }

    #[test]
    fn v2_report_pins_backend_and_seven_stages() {
        let json = serde_json::to_value(Report::new(None)).unwrap();
        let mut keys = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "backend",
                "budget_exhausted",
                "budget_overshoot_bytes",
                "build",
                "checkpoint",
                "completed_event_indexes",
                "elapsed_ns",
                "incomplete_reasons",
                "integrity",
                "io_budget_bytes",
                "manifest",
                "process_lifetime_written_bytes",
                "samples",
                "scenario",
                "schema_version",
                "sqlite",
                "stages",
                "status",
                "summary",
                "ui_response_missing_reason",
                "ui_response_ns",
                "wal_peak_bytes",
                "wal_peak_method",
                "wall_limit_ms",
                "whole_run_io_after",
                "whole_run_io_before",
                "whole_run_process_written_bytes",
            ]
        );
        let backend = serde_json::to_value(Backend {
            kind: "blob",
            mode_version: None,
        })
        .unwrap();
        assert_eq!(json["backend"], backend);
        let mut stages = json["stages"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        stages.sort_unstable();
        assert_eq!(
            stages,
            [
                "checkpoint",
                "close",
                "conversion",
                "final_save",
                "hot",
                "restart",
                "setup",
            ]
        );
        let conversion = json["stages"]["conversion"].as_object().unwrap();
        assert_eq!(conversion["attempted"], false);
        assert!(conversion
            .values()
            .all(|value| value.is_null() || value.as_bool() == Some(false)));
    }

    #[test]
    fn v2_blob_chunk_counters_are_null_with_backend_reason() {
        let values = SampleValues::from(SaveSample::default());
        assert_eq!(values.blocks_written, None);
        assert_eq!(values.chunks_written, None);
        assert_eq!(values.chunk_bytes_written, None);
        assert_eq!(values.chunks_written_missing_reason, Some("backend:blob"));
        assert_eq!(
            values.chunk_bytes_written_missing_reason,
            Some("backend:blob")
        );
        let json = serde_json::to_value(values).unwrap();
        assert!(json["chunks_written"].is_null() && json["chunk_bytes_written"].is_null());
    }

    #[test]
    fn v2_summary_totals_gain_block_and_chunk_counters() {
        let sample = |values: Option<SampleValues>| Sample {
            event_index: 0,
            session_index: 0,
            logical_ms: 0,
            started_ms: 0,
            completed_ms: 1,
            save_ns: 7,
            io_before: None,
            io_after: None,
            process_written_bytes: None,
            values,
        };
        let mut values = SampleValues::from(SaveSample::default());
        values.blocks_written = Some(6);
        let summary = summarize(&[sample(Some(values.clone())), sample(Some(values))]);
        assert_eq!(summary["totals"]["blocks_written"], 12);
        assert_eq!(summary["max"]["blocks_written"], 6);
        assert!(summary["totals"]["chunks_written"].is_null());
        assert!(summary["totals"]["chunk_bytes_written"].is_null());
        assert!(summary["max"]["chunks_written"].is_null());
        // No known value stays null; a fabricated zero total is a lie.
        let bare = summarize(&[sample(None)]);
        assert!(bare["totals"]["blocks_written"].is_null());
        assert!(bare["max"]["blocks_written"].is_null());
    }

    #[test]
    fn save_event_populates_blob_blocks_written() {
        let (blocks, _) = fixture(8192, "append-1m").unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("synthetic.db");
        let store = Store::open_monocode(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute("INSERT INTO sessions(id,cwd,harness,model,runtime_mode,title,blocks_json,created_at,updated_at,queued_messages_json)
            VALUES('fixed-session-0','synthetic','synthetic','synthetic:fixed','supervised','Synthetic',?1,?2,?2,?3)",
            params![serde_json::to_string(&blocks).unwrap(),FIXED_TIME.min(i64::MAX as u64) as i64,queue().to_string()]).unwrap();
        let id = store.sessions().unwrap()[0].id;
        let mut transcript = store.load(id).unwrap();
        let store = Mutex::new(store);
        let event = events("append-1m", 1).remove(0);
        let sample = save_event(
            &store,
            &mut transcript,
            id,
            &event,
            Instant::now(),
            blocks.len(),
            false,
        )
        .unwrap();
        let values = sample.values.unwrap();
        // The blob backend rewrites the whole session block array per save.
        assert_eq!(values.blocks_written, Some(blocks.len() as u64));
        assert_eq!(values.chunks_written, None);
        assert_eq!(values.chunk_bytes_written, None);
        assert_eq!(values.chunks_written_missing_reason, Some("backend:blob"));
        assert_eq!(
            values.chunk_bytes_written_missing_reason,
            Some("backend:blob")
        );
    }

    #[test]
    fn manifest_event_keys_stay_v1_byte_compatible() {
        let planned = serde_json::to_value(events("append-1m", 1)).unwrap();
        let mut keys = planned[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "append_bytes",
                "append_sha256",
                "index",
                "logical_ms",
                "operation",
                "reason",
                "session_index",
            ]
        );
        // The v2 fields live on samples/report only; the workload hash input
        // (events + save_parameters) must not move.
        assert_eq!(
            json_hash(&events("append-1m", 1)).unwrap(),
            json_hash(&events("append-1m", 1)).unwrap()
        );
    }

    #[test]
    fn fixture_and_workload_are_deterministic_utf8_bytes() {
        let (first, _) = fixture(8192, "append-1m").unwrap();
        let (second, _) = fixture(8192, "append-1m").unwrap();
        assert_eq!(json_hash(&first).unwrap(), json_hash(&second).unwrap());
        assert_eq!(serde_json::to_vec(&first[..5]).unwrap().len(), 8192);
        let workload = events("append-1m", 1);
        assert_eq!(
            workload
                .iter()
                .map(|event| event.logical_ms)
                .collect::<Vec<_>>(),
            [0, 650, 1300, 1950, 2600]
        );
        assert!(workload[..4]
            .iter()
            .all(|event| event.append_bytes == 1024
                && event.append_sha256 == hash(&vec![b'x'; 1024])));
        assert_eq!(
            json_hash(&workload).unwrap(),
            json_hash(&events("append-1m", 1)).unwrap()
        );
    }

    #[test]
    fn cli_never_accepts_a_database_path() {
        assert!(parse_scenario(&["--database".into(), "synthetic.db".into()]).is_err());
        assert!(parse_scenario(&[
            "--scenario".into(),
            "append-1m".into(),
            "synthetic.db".into()
        ])
        .is_err());
        for scenario in SCENARIOS {
            assert_eq!(
                parse_scenario(&["--scenario".into(), scenario.into()]).unwrap(),
                (scenario, false)
            );
        }
    }

    #[test]
    fn backend_flag_opts_into_native_chunks_only() {
        // The flag pairs with the scenario in either order and records nothing
        // but itself: the manifest inputs never see it.
        assert_eq!(
            parse_scenario(&[
                "--scenario".into(),
                "large-tool".into(),
                "--backend".into(),
                "native-chunks".into()
            ])
            .unwrap(),
            ("large-tool", true)
        );
        assert_eq!(
            parse_scenario(&[
                "--backend".into(),
                "native-chunks".into(),
                "--scenario".into(),
                "append-40m".into()
            ])
            .unwrap(),
            ("append-40m", true)
        );
        // Anything else refuses: unknown or missing values, duplicates, a
        // misspelled backend, or a bare backend without a scenario.
        for args in [
            vec!["--scenario", "append-1m", "--backend", "blob"],
            vec![
                "--scenario",
                "append-1m",
                "--backend",
                "native-chunks",
                "--backend",
                "native-chunks",
            ],
            vec!["--scenario", "append-1m", "--scenario", "append-1m"],
            vec!["--scenario", "append-1m", "--backend"],
            vec!["--backend", "native-chunks"],
            vec!["--scenario", "not-a-scenario", "--backend", "native-chunks"],
            vec!["--backend", "native-chunks", "--extra", "1"],
        ] {
            let args = args.into_iter().map(String::from).collect::<Vec<_>>();
            assert!(parse_scenario(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn native_backend_flag_reports_store_mode_and_real_counters() {
        let (blocks, _) = fixture(8192, "append-1m").unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("synthetic.db");
        let mut store = Store::open_monocode(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let raw = serde_json::to_string(&blocks).unwrap();
        conn.execute("INSERT INTO sessions(id,cwd,harness,model,runtime_mode,title,blocks_json,created_at,updated_at,has_user_message,queued_messages_json)
            VALUES('fixed-session-0','synthetic','synthetic','synthetic:fixed','supervised','Synthetic',?1,?2,?2,1,?3)",
            params![raw.clone(),FIXED_TIME.min(i64::MAX as u64) as i64,queue().to_string()]).unwrap();
        // The run's opt-in: the mode marker, then a full store of the session's
        // fixture blocks through the chunked backend's own write path.
        conn.execute(
            "INSERT INTO native_sessions(session_id,mode_version,session_revision) VALUES('fixed-session-0',?1,0)",
            params![i64::from(NATIVE_CHUNKS_MODE_VERSION)],
        )
        .unwrap();
        let id = store.sessions().unwrap()[0].id;
        assert_eq!(
            store.storage_mode_version(id).unwrap(),
            Some(NATIVE_CHUNKS_MODE_VERSION)
        );
        store.store_blocks(id, &blocks).unwrap();
        // The opt-in round-trips the fixture exactly — ids, receipts and
        // unknown metadata included — through the backend the fingerprint
        // reads, so the run's `initial_fixture_matches` is a real check.
        assert_eq!(store.stored_blocks(id).unwrap(), blocks);
        // With the marker in place a load reads the native tables, so the
        // transcript loads only after the opt-in stored them.
        let mut transcript = store.load(id).unwrap();
        // The desktop blob keeps the fixture bytes exactly as inserted: the
        // chunked backend owns the transcript without rewriting them.
        let stored: String = conn
            .query_row(
                "SELECT blocks_json FROM sessions WHERE id='fixed-session-0'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, raw);
        let store = Mutex::new(store);
        let event = events("append-1m", 1).remove(0);
        let sample = save_event(
            &store,
            &mut transcript,
            id,
            &event,
            Instant::now(),
            blocks.len(),
            true,
        )
        .unwrap();
        let values = sample.values.unwrap();
        // The chunked backend's own counters, verbatim: one dirty block, at
        // least the one chunk its save moved, and no missing reasons.
        assert_eq!(values.blocks_written, Some(1));
        assert!(values.chunks_written.is_some_and(|count| count >= 1));
        assert!(values.chunk_bytes_written.is_some_and(|count| count > 0));
        assert_eq!(values.chunks_written_missing_reason, None);
        assert_eq!(values.chunk_bytes_written_missing_reason, None);
    }

    #[test]
    fn stats_do_not_serialize_payloads() {
        let sample = serde_json::to_value(SampleValues::from(SaveSample::default())).unwrap();
        assert_eq!(
            sample
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "blocks_written",
                "changed_rows",
                "chunk_bytes_written",
                "chunk_bytes_written_missing_reason",
                "chunks_written",
                "chunks_written_missing_reason",
                "commit_count",
                "commit_ns",
                "encoding_ns",
                "history_bytes_read",
                "lock_held_ns",
                "lock_wait_ns",
                "projection_ns",
                "reason",
                "serialized_bytes",
                "succeeded"
            ]
        );
        assert!(sample
            .as_object()
            .unwrap()
            .iter()
            .all(|(key, value)| key == "reason"
                || key.ends_with("_missing_reason")
                || !value.is_string()));
        assert_eq!(sample["chunks_written_missing_reason"], "backend:blob");
        assert_eq!(sample["chunk_bytes_written_missing_reason"], "backend:blob");
    }

    #[test]
    fn small_scenarios_preserve_rows_unknown_metadata_and_receipts() {
        for scenario in [
            "append-1m",
            "same-value",
            "early-tool",
            "large-tool",
            "long-answer",
        ] {
            let (mut blocks, _) = fixture(8192, scenario).unwrap();
            // Correctness uses small payloads; the real manifest keeps 5 MiB.
            if scenario == "large-tool" {
                blocks[5]["tool"]["nativeInput"] = json!("x".repeat(8192));
            }
            if scenario == "long-answer" {
                blocks[5]["text"] = json!("a".repeat(8192));
            }
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("synthetic.db");
            let mut store = Store::open_monocode(&path).unwrap();
            let conn = Connection::open(&path).unwrap();
            conn.execute("INSERT INTO sessions(id,cwd,harness,model,runtime_mode,title,blocks_json,created_at,updated_at,queued_messages_json)
                VALUES('fixed-session-0','synthetic','synthetic','synthetic:fixed','supervised','Synthetic',?1,?2,?2,?3)",
                params![serde_json::to_string(&blocks).unwrap(),FIXED_TIME.min(i64::MAX as u64) as i64,queue().to_string()]).unwrap();
            let id = store.sessions().unwrap()[0].id;
            let before = fingerprint(&conn, &store, &[id]).unwrap();
            let mut transcript = store.load(id).unwrap();
            for event in events(scenario, 1) {
                let from = mutate(&mut transcript, &event).unwrap();
                store
                    .write_rows(id, from, &transcript.rows[from..])
                    .unwrap();
                assert!(store.take_save_sample().unwrap().succeeded);
            }
            let after = fingerprint(&conn, &store, &[id]).unwrap();
            assert_eq!(
                after.rows_sha256,
                json_hash(&vec![&transcript.rows]).unwrap(),
                "{scenario}"
            );
            assert_eq!(
                before.completed_history_sha256, after.completed_history_sha256,
                "{scenario}"
            );
            assert_eq!(
                before.unknown_metadata_sha256, after.unknown_metadata_sha256,
                "{scenario}"
            );
            assert_eq!(before.receipts_sha256, after.receipts_sha256, "{scenario}");
            assert_eq!(before.queue_sha256, after.queue_sha256, "{scenario}");
            drop(conn);
            drop(store);
            let store = Store::open_monocode(&path).unwrap();
            let conn = Connection::open(&path).unwrap();
            let restarted = fingerprint(&conn, &store, &[id]).unwrap();
            assert!(after == restarted, "{scenario}");
            conn.execute(
                "UPDATE sessions SET queued_messages_json='[]' WHERE id='fixed-session-0'",
                [],
            )
            .unwrap();
            assert_ne!(
                fingerprint(&conn, &store, &[id]).unwrap().queue_sha256,
                after.queue_sha256
            );
        }
    }

    #[test]
    fn unknown_metadata_and_receipts_are_fingerprinted() {
        let mut first = vec![];
        let mut second = vec![];
        unknown(
            &json!({"tool":{"futureToolField":1},"futureMetadata":{"keep":true}}),
            "",
            &mut first,
        );
        unknown(
            &json!({"tool":{"futureToolField":2},"futureMetadata":{"keep":true}}),
            "",
            &mut second,
        );
        assert_ne!(json_hash(&first).unwrap(), json_hash(&second).unwrap());
    }

    #[test]
    fn supplemental_flag_parses_alone_and_with_the_backend_flag() {
        // Legal without the backend flag, with it, and in any order.
        assert_eq!(
            parse_scenario(&[
                "--scenario".into(),
                "append-1m".into(),
                "--supplemental-saves".into(),
                "120".into()
            ])
            .unwrap(),
            Cli {
                scenario: "append-1m",
                native_chunks: false,
                supplemental_saves: Some(120)
            }
        );
        assert_eq!(
            parse_scenario(&[
                "--scenario".into(),
                "append-1m".into(),
                "--backend".into(),
                "native-chunks".into(),
                "--supplemental-saves".into(),
                "100".into()
            ])
            .unwrap(),
            Cli {
                scenario: "append-1m",
                native_chunks: true,
                supplemental_saves: Some(100)
            }
        );
        assert_eq!(
            parse_scenario(&[
                "--supplemental-saves".into(),
                "7".into(),
                "--backend".into(),
                "native-chunks".into(),
                "--scenario".into(),
                "append-1m".into()
            ])
            .unwrap(),
            Cli {
                scenario: "append-1m",
                native_chunks: true,
                supplemental_saves: Some(7)
            }
        );
        // The (scenario, backend) projection the pre-flag assertions rely on.
        assert_eq!(
            parse_scenario(&[
                "--scenario".into(),
                "append-1m".into(),
                "--supplemental-saves".into(),
                "120".into()
            ])
            .unwrap(),
            ("append-1m", false)
        );
        // Missing value, non-numeric, zero, negative (usize rejects it) and
        // duplicate all refuse; the flag alone is no invocation either.
        for args in [
            vec!["--scenario", "append-1m", "--supplemental-saves"],
            vec!["--scenario", "append-1m", "--supplemental-saves", "many"],
            vec!["--scenario", "append-1m", "--supplemental-saves", "0"],
            vec!["--scenario", "append-1m", "--supplemental-saves", "-5"],
            vec![
                "--scenario",
                "append-1m",
                "--supplemental-saves",
                "100",
                "--supplemental-saves",
                "100",
            ],
            vec!["--supplemental-saves", "100"],
        ] {
            let args = args.into_iter().map(String::from).collect::<Vec<_>>();
            assert!(parse_scenario(&args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn manifest_stays_byte_identical_without_the_supplemental_flag() {
        // The fixed workload's save parameters serialize to the exact bytes the
        // manifest always carried (a serde_json Value orders its keys).
        assert_eq!(
            serde_json::to_string(&save_parameters(2600, 1)).unwrap(),
            r#"{"append_bytes":1024,"busy_timeout_ms":5000,"database":"new_temporary_blob_schema","final_logical_ms":2600,"final_save":"last_message_suffix_even_if_unchanged","parallel_policy":"barrier_per_logical_tick_shared_store_mutex","parallel_sessions":1,"routine_interval_ms":650}"#
        );
        let manifest = |supplemental: Option<usize>, planned: Vec<Event>, parameters: Value| {
            serde_json::to_value(Manifest {
                fixture_sha256: hash(b"fixture"),
                fixture_utf8_bytes: 8192,
                completed_history_utf8_bytes_per_session: MIB,
                active_payload_utf8_bytes_per_session: 0,
                initial_blocks_per_session: 7,
                initial_rows_per_session: 7,
                session_count: 1,
                fixture_version: 1,
                workload_sha256: json_hash(&json!({"events":planned,"save_parameters":parameters}))
                    .unwrap(),
                events: planned,
                save_parameters: parameters,
                supplemental: supplemental.map(|_| true),
                supplemental_saves: supplemental,
            })
            .unwrap()
        };
        // Without the flag neither supplemental key appears: the manifest key
        // set stays exactly the pre-supplemental one.
        let plain = manifest(None, events("append-1m", 1), save_parameters(2600, 1));
        let mut keys = plain
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "active_payload_utf8_bytes_per_session",
                "completed_history_utf8_bytes_per_session",
                "events",
                "fixture_sha256",
                "fixture_utf8_bytes",
                "fixture_version",
                "initial_blocks_per_session",
                "initial_rows_per_session",
                "save_parameters",
                "session_count",
                "workload_sha256",
            ]
        );
        // With the flag the manifest labels itself and plans N small saves at
        // i·650 ms plus the final save at N·650 ms.
        let labeled = manifest(
            Some(7),
            supplemental_events(7, 1),
            save_parameters(7 * 650, 1),
        );
        assert_eq!(labeled["supplemental"], true);
        assert_eq!(labeled["supplemental_saves"], 7);
        assert_eq!(labeled["events"].as_array().unwrap().len(), 8);
        assert_eq!(labeled["events"][6]["logical_ms"], 6 * 650);
        assert_eq!(labeled["events"][7]["logical_ms"], 7 * 650);
        assert_eq!(labeled["events"][7]["operation"], "final_save");
        assert_eq!(labeled["save_parameters"]["final_logical_ms"], 7 * 650);
    }

    #[test]
    fn p95_is_nearest_rank_on_sorted_durations() {
        assert_eq!(p95_nearest_rank(&[]), None);
        assert_eq!(
            p95_nearest_rank(&(1..=100u64).collect::<Vec<_>>()),
            Some(95)
        );
        // n = 20 → ceil(0.95·20) = 19: the 19th smallest, not the maximum.
        assert_eq!(p95_nearest_rank(&(1..=20u64).collect::<Vec<_>>()), Some(19));
        // n = 4 → rank 4, the maximum of a tiny sample.
        assert_eq!(p95_nearest_rank(&[10, 20, 20, 30]), Some(30));
        // Nearest rank, never interpolation: the 19th of 20 values is 9, where
        // a linear interpolation between the 19th and 20th would exceed it.
        assert_eq!(
            p95_nearest_rank(&[5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 9, 100]),
            Some(9)
        );
    }

    #[test]
    fn supplemental_completeness_requires_every_planned_event_in_order() {
        let planned = supplemental_events(2, 1);
        assert_eq!(
            planned.iter().map(|event| event.index).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert!(workload_fully_sampled(&[0, 1, 2], 3, &planned));
        // Fewer samples, out-of-order completion, or an extra sample all
        // refuse completeness — the fixed workload's rule, count-adapted.
        assert!(!workload_fully_sampled(&[0, 1], 2, &planned));
        assert!(!workload_fully_sampled(&[0, 2, 1], 3, &planned));
        assert!(!workload_fully_sampled(&[0, 1, 2, 3], 4, &planned));
        let fixed = events("append-1m", 1);
        assert!(workload_fully_sampled(&[0, 1, 2, 3, 4], 5, &fixed));
        assert!(!workload_fully_sampled(&[0, 1, 2], 3, &fixed));
    }

    #[test]
    fn supplemental_report_object_ranks_routine_save_ns_only() {
        let sample = |save_ns: u64, reason: SaveReason| {
            let mut values = SampleValues::from(SaveSample::default());
            values.reason = reason;
            Sample {
                event_index: 0,
                session_index: 0,
                logical_ms: 0,
                started_ms: 0,
                completed_ms: 1,
                save_ns,
                io_before: None,
                io_after: None,
                process_written_bytes: None,
                values: Some(values),
            }
        };
        let mut report = Report::new(None);
        report.supplemental_saves = Some(4);
        report.samples = vec![
            sample(10, SaveReason::Routine),
            sample(20, SaveReason::Routine),
            sample(30, SaveReason::Routine),
            sample(40, SaveReason::Routine),
            // The final save is far larger and must never feed the percentile.
            sample(9_000_000, SaveReason::Final),
        ];
        report.finish(Instant::now());
        let supplemental = report.supplemental.as_ref().unwrap();
        assert!(supplemental.label);
        assert_eq!(supplemental.samples, 4);
        // ceil(0.95·4) = 4 → the largest routine save_ns.
        assert_eq!(supplemental.p95_ns, Some(40));
        assert_eq!(supplemental.max_ns, Some(40));
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["supplemental"]["definition"], SUPPLEMENTAL_DEFINITION);
        assert_eq!(
            json["summary"]["p95_missing_reason"],
            "p95_reported_in_supplemental_object"
        );
        // Without the flag no report ever grows the object or moves the
        // generic summary reason.
        let mut plain = Report::new(None);
        plain.samples = vec![sample(10, SaveReason::Routine)];
        plain.finish(Instant::now());
        assert!(plain.supplemental.is_none());
        assert!(serde_json::to_value(&plain)
            .unwrap()
            .get("supplemental")
            .is_none());
        assert_eq!(
            plain.summary["p95_missing_reason"],
            "single_fixed_workload_fewer_than_100_samples"
        );
    }
}
