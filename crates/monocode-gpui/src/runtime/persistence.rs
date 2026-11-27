//! UI batches stay responsive while the shared JSON transcript is saved at a
//! bounded cadence. Delivery receipts and settled turns always flush immediately.
use super::*;
use monocode_core::store::DirtyBlock;
use std::time::Instant;

// Match the desktop's routine-save window. Accepted messages and settled turns
// still commit immediately, independently of this streaming debounce.
const SHARED_SAVE_INTERVAL: Duration = Duration::from_millis(650);

pub(super) struct PendingSave {
    from: Option<usize>,
    /// Non-adjacent dirty block ranges for a chunked-mode session, recorded
    /// instead of `from` so one flush covers an early edit plus a streaming
    /// tail. The two never mix for one session: blob sessions keep using the
    /// `from` suffix window.
    dirty: Vec<DirtyBlock>,
    context_dirty: bool,
    last_flush: Instant,
    interval: Duration,
    context_owner: Option<(String, String)>,
}

impl PendingSave {
    pub(super) fn new(shared: bool, now: Instant) -> Self {
        Self {
            from: None,
            dirty: Vec::new(),
            context_dirty: false,
            last_flush: now,
            interval: if shared {
                SHARED_SAVE_INTERVAL
            } else {
                Duration::ZERO
            },
            context_owner: None,
        }
    }

    pub(super) fn own_context(&mut self, provider: &str, account: &str) {
        self.context_owner = Some((provider.into(), account.into()));
    }

    pub(super) fn changed(&mut self, row: usize) {
        self.from = Some(self.from.map_or(row, |old| old.min(row)));
    }

    /// Record one dirty block range (inclusive absolute rows) for a
    /// chunked-mode session. Unlike `changed`'s suffix window, the ranges need
    /// not be adjacent; the store writes only the blocks they touch.
    pub(super) fn dirty_block(&mut self, block: DirtyBlock) {
        self.dirty.push(block);
    }

    pub(super) fn context_changed(&mut self) {
        self.context_dirty = true;
    }

    /// The earliest row any pending change touched, whichever tracking
    /// mechanism holds it.
    fn pending_from(&self) -> Option<usize> {
        let dirty = self
            .dirty
            .iter()
            .map(|block| block.first_row)
            .filter(|row| *row >= 0)
            .min()
            .map(|row| row as usize);
        match (self.from, dirty) {
            (Some(from), Some(dirty)) => Some(from.min(dirty)),
            (from, dirty) => from.or(dirty),
        }
    }

    /// The disjoint row ranges pending a flush, as slices of `transcript`.
    fn pending_ranges<'a>(&self, transcript: &'a Transcript) -> Vec<(usize, &'a [Row])> {
        if self.dirty.is_empty() {
            return self
                .from
                .map(|from| (from, &transcript.rows[from..]))
                .into_iter()
                .collect();
        }
        let mut ranges: Vec<(usize, usize)> = self
            .dirty
            .iter()
            .filter_map(|block| {
                (block.first_row >= 0).then_some((
                    block.first_row as usize,
                    (block.last_row + 1).max(0) as usize,
                ))
            })
            .collect();
        if let Some(from) = self.from {
            ranges.push((from, transcript.rows.len()));
        }
        ranges.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (start, end) in ranges {
            match merged.last_mut() {
                Some((_, last_end)) if start <= *last_end => *last_end = (*last_end).max(end),
                _ => merged.push((start, end)),
            }
        }
        merged
            .into_iter()
            .map(|(start, end)| {
                let end = end.min(transcript.rows.len());
                (start.min(end), &transcript.rows[start.min(end)..end])
            })
            .filter(|(_, rows)| !rows.is_empty())
            .collect()
    }

    pub(super) fn due(&self, boundary: bool, now: Instant) -> bool {
        (self.pending_from().is_some() || self.context_dirty)
            && (boundary || now.saturating_duration_since(self.last_flush) >= self.interval)
    }

    pub(super) fn flush(
        &mut self,
        store: &Database,
        session: i64,
        transcript: &Transcript,
        context: &monocode_core::context::Usage,
        reason: SaveReason,
        now: Instant,
    ) -> Result<(), String> {
        if self.pending_from().is_none() && !self.context_dirty {
            return Ok(());
        }
        let wait = Instant::now();
        let mut store = locked(store)?;
        let acquired = Instant::now();
        let _ = store.take_save_sample();
        let ranges = self.pending_ranges(transcript);
        let result: Result<(), String> = (|| {
            if !ranges.is_empty() {
                // One call, one transaction: an early edit and a streaming
                // tail land together, and a blob session's suffix behaves
                // exactly as the single-range write always did.
                store.write_row_ranges(session, &ranges)?;
            }
            if self.context_dirty {
                let selected = if let Some((provider, account)) = &self.context_owner {
                    store
                        .agent_settings(session)?
                        .is_some_and(|settings| settings.account.id == *account)
                        && store
                            .sessions()?
                            .iter()
                            .any(|current| current.id == session && current.provider == *provider)
                } else {
                    true
                };
                if selected {
                    store.save_context_usage(session, context)?;
                }
            }
            Ok(())
        })();
        // Take before releasing this save's lock, including the failure path.
        let sample = store.take_save_sample();
        let held = acquired.elapsed();
        drop(store);
        if let Some(mut sample) = sample {
            sample.reason = reason;
            sample.lock_wait = acquired.saturating_duration_since(wait);
            sample.lock_held = held;
            sample.succeeded &= result.is_ok();
            crate::persistence_log::record(sample);
        }
        result?;
        // Keep pending changes after a failed write so final cleanup retries.
        self.from = None;
        self.dirty.clear();
        self.context_dirty = false;
        self.last_flush = now;
        Ok(())
    }

    pub(super) fn flush_final(
        &mut self,
        store: &Database,
        session: i64,
        transcript: &Transcript,
        context: &monocode_core::context::Usage,
    ) -> (Result<(), String>, bool) {
        let result = self.flush(
            store,
            session,
            transcript,
            context,
            SaveReason::Final,
            Instant::now(),
        );
        let Err(error) = result else {
            return (result, false);
        };
        let ranges = self.pending_ranges(transcript);
        if !ranges
            .iter()
            .flat_map(|(_, rows)| rows.iter())
            .any(|row| row.role == "assistant" && !row.text.is_empty())
        {
            return (Err(error), false);
        }
        match recovery::retain(store, session, &transcript.rows, &ranges) {
            Ok(Some(true)) => (
                Err(format!(
                    "{error} · The reply is kept and will be saved again."
                )),
                true,
            ),
            Ok(Some(false)) => (
                Err(format!(
                    "{error} · The reply is kept in memory and will be saved again. Keep the app open until it saves."
                )),
                true,
            ),
            Ok(None) => (Err(error), false),
            Err(recovery) => (
                Err(format!(
                    "{error} · Could not preserve the reply: {recovery}"
                )),
                false,
            ),
        }
    }

    pub(super) fn accept(
        &mut self,
        store: &Database,
        session: i64,
        from: usize,
        transcript: &Transcript,
        message: Option<&str>,
        now: Instant,
    ) -> Result<(), String> {
        let from = self.pending_from().map_or(from, |old| old.min(from));
        let wait = Instant::now();
        let mut store = locked(store)?;
        let acquired = Instant::now();
        let result =
            store.write_rows_with_message(session, from, &transcript.rows[from..], message);
        let sample = store.take_save_sample();
        let held = acquired.elapsed();
        drop(store);
        if let Some(mut sample) = sample {
            sample.reason = SaveReason::Acceptance;
            sample.lock_wait = acquired.saturating_duration_since(wait);
            sample.lock_held = held;
            crate::persistence_log::record(sample);
        }
        result?;
        self.from = None;
        self.dirty.clear();
        self.last_flush = now;
        Ok(())
    }
}

/// Publish ordinary streaming rows before their save competes for the Store
/// lock: the display never waits behind a parked save (spec L108). Acceptance
/// receipts, visual saved replies and finish states are not routed through
/// here — they still confirm only after the commit succeeds.
pub(super) fn publish_then_flush(
    pending: &mut PendingSave,
    store: &Database,
    session: i64,
    transcript: &Transcript,
    context: &monocode_core::context::Usage,
    boundary: bool,
    publish: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let now = Instant::now();
    publish()?;
    if pending.due(boundary, now) {
        pending.flush(
            store,
            session,
            transcript,
            context,
            if boundary {
                SaveReason::Boundary
            } else {
                SaveReason::Routine
            },
            now,
        )
    } else {
        Ok(())
    }
}

pub(super) fn save_turn_receipt(
    store: &Database,
    session: i64,
    turn: usize,
    info: monocode_core::agent::TurnInfo,
) -> Result<(), String> {
    let store = locked(store)?;
    // The picker/composer may have changed during the run. Update only its
    // immutable turn receipts, using the latest settings revision.
    let mut settings = store.agent_settings(session)?.unwrap_or_default();
    settings.turns.insert(turn, info);
    store.save_agent_settings(session, &settings)
}

#[cfg(test)]
mod persistence_safety_tests {
    use super::*;

    #[test]
    fn initial_acceptance_preserves_the_first_routine_deadline() {
        let core = Store::open_monocode(std::path::Path::new(":memory:")).unwrap();
        let session = core.create("codex", "/project").unwrap();
        let store = Arc::new(Mutex::new(core));
        let mut transcript = Transcript::default();
        transcript.begin("user");
        transcript.append("Synthetic");
        transcript.begin("assistant");
        let now = Instant::now();
        let mut pending = PendingSave::new(true, now);
        pending
            .accept(&store, session.id, 0, &transcript, None, now)
            .unwrap();
        pending.changed(transcript.append("x"));
        assert_eq!(pending.last_flush, now);
        assert!(!pending.due(false, now + Duration::from_millis(649)));
        assert!(pending.due(false, now + Duration::from_millis(650)));
        assert!(locked(&store).unwrap().take_save_sample().is_none());
    }

    #[test]
    fn failed_commit_keeps_pending_rows_and_acceptance_retryable() {
        let mut core = Store::open_monocode(std::path::Path::new(":memory:")).unwrap();
        let session = core.create("codex", "/project").unwrap();
        let mut transcript = Transcript::default();
        transcript.begin("user");
        transcript.append("Synthetic");
        transcript.begin("assistant");
        core.write_rows(session.id, 0, &transcript.rows).unwrap();
        let store = Arc::new(Mutex::new(core));
        let now = Instant::now();
        let mut pending = PendingSave::new(true, now);
        pending.changed(transcript.append("中文🙂"));
        locked(&store)
            .unwrap()
            .fail_next_save_commit_for_test()
            .unwrap();
        assert!(
            pending
                .flush(
                    &store,
                    session.id,
                    &transcript,
                    &Default::default(),
                    SaveReason::Final,
                    now
                )
                .is_err()
        );
        assert!(pending.from.is_some());
        assert!(pending.due(true, now));
        pending
            .flush(
                &store,
                session.id,
                &transcript,
                &Default::default(),
                SaveReason::Final,
                now,
            )
            .unwrap();
        assert!(pending.from.is_none());
        assert_eq!(
            locked(&store).unwrap().load(session.id).unwrap().rows,
            transcript.rows
        );
        assert!(
            locked(&store).unwrap().take_save_sample().is_none(),
            "sample is consumed inside the save lock"
        );
        let from = transcript.begin("user");
        transcript.append("Next synthetic");
        pending.changed(from);
        locked(&store)
            .unwrap()
            .fail_next_save_commit_for_test()
            .unwrap();
        assert!(
            pending
                .accept(
                    &store,
                    session.id,
                    from,
                    &transcript,
                    Some("fixed-message"),
                    now
                )
                .is_err()
        );
        assert_eq!(pending.from, Some(from));
        pending
            .accept(
                &store,
                session.id,
                from,
                &transcript,
                Some("fixed-message"),
                now,
            )
            .unwrap();
        assert!(pending.from.is_none());
    }

    /// A saved conversation plus a streaming tail, as the safety tests use it.
    fn safety_conversation() -> (Database, i64, Transcript) {
        let mut store = Store::open_monocode(std::path::Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap();
        let mut transcript = Transcript::default();
        transcript.begin("user");
        transcript.append("Prompt");
        transcript.begin("assistant");
        store.write_rows(session.id, 0, &transcript.rows).unwrap();
        (Arc::new(Mutex::new(store)), session.id, transcript)
    }

    /// Park the Store mutex on a real thread (the batch-1 barrier style): the
    /// save cannot proceed until `release`, and a lost thread fails loudly
    /// instead of hanging the suite.
    struct ParkedStore {
        release: std::sync::mpsc::Sender<()>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl ParkedStore {
        fn park(store: &Database) -> (Self, std::sync::mpsc::Receiver<()>) {
            let (held, held_rx) = std::sync::mpsc::channel();
            let (release, release_rx) = std::sync::mpsc::channel();
            let store = store.clone();
            let thread = std::thread::spawn(move || {
                let _guard = locked(&store).unwrap();
                held.send(()).unwrap();
                release_rx
                    .recv_timeout(Duration::from_secs(30))
                    .expect("parked store released");
            });
            (
                Self {
                    release,
                    thread: Some(thread),
                },
                held_rx,
            )
        }
        fn release(mut self) {
            if let Some(thread) = self.thread.take() {
                self.release.send(()).unwrap();
                thread.join().unwrap();
            }
        }
    }
    impl Drop for ParkedStore {
        fn drop(&mut self) {
            if let Some(thread) = self.thread.take() {
                let _ = self.release.send(());
                let _ = thread.join();
            }
        }
    }

    fn streaming_update(start: usize, rows: Vec<Row>) -> Update {
        Update {
            worktree_cwd: None,
            commands: None,
            provider_turn_id: None,
            turn_ready: false,
            follow_up_delivered: Vec::new(),
            follow_up_errors: Vec::new(),
            delivered: None,
            start,
            rows,
            provider_id: None,
            model: None,
            config_model: None,
            config_settings: None,
            title: None,
            context: None,
            metrics: None,
            usage_limit: None,
            background: None,
            activity: None,
            provider_event: true,
            approvals: Vec::new(),
            approvals_resolved: Vec::new(),
            questions: Vec::new(),
            questions_resolved: Vec::new(),
            handoff: None,
            status: "Streaming".into(),
            finished: false,
            reply_pending: false,
        }
    }

    #[test]
    fn display_publishes_before_save_completes() {
        let (store, id, mut transcript) = safety_conversation();
        let now = Instant::now();
        let mut pending = PendingSave::new(true, now);
        let tail = transcript.append("Tail token 中文");
        pending.changed(tail);
        let (tx, rx) = async_channel::bounded(8);
        let cancel = Arc::new(AtomicBool::new(false));
        let (parked, held) = ParkedStore::park(&store);
        held.recv_timeout(Duration::from_secs(10)).unwrap();
        // The save is due now, but the Store mutex is parked: the display
        // update must still publish before the save can complete.
        let worker_store = store.clone();
        let worker_tx = tx.clone();
        let worker_cancel = cancel.clone();
        let worker = std::thread::spawn(move || {
            let mut worker_transcript = Transcript::default();
            worker_transcript.begin("user");
            worker_transcript.append("Prompt");
            let worker_tail = worker_transcript.begin("assistant");
            worker_transcript.append("Tail token 中文");
            let update =
                streaming_update(worker_tail, worker_transcript.rows[worker_tail..].to_vec());
            let mut pending = pending;
            let result = publish_then_flush(
                &mut pending,
                &worker_store,
                id,
                &worker_transcript,
                &Default::default(),
                true,
                move || publish(&worker_tx, &worker_cancel, update),
            );
            (pending, result)
        });
        // async_channel has no timed blocking receive, so a reader thread
        // bridges it to a std channel with one.
        let (update_tx, update_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = update_tx.send(rx.recv_blocking().expect("publish"));
        });
        let update = update_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the display publishes while the Store lock is parked");
        assert_eq!(update.rows.last().unwrap().text, "Tail token 中文");
        assert!(!worker.is_finished(), "the save still waits on the lock");
        // Only now can the save run and commit.
        parked.release();
        let (pending, result) = worker.join().unwrap();
        result.unwrap();
        assert!(pending.from.is_none());
        assert_eq!(
            locked(&store).unwrap().load(id).unwrap().rows,
            transcript.rows
        );
    }

    #[test]
    fn non_adjacent_dirty_ranges_flush_as_one_save() {
        let (store, id, mut transcript) = safety_conversation();
        let now = Instant::now();
        let mut pending = PendingSave::new(true, now);
        // Three messages the first save lands; then an in-place edit of the
        // early one and of the tail, with the middle untouched: the two
        // non-adjacent dirty ranges coalesce into one flush whose write set
        // leaves the middle row out.
        let early = transcript.begin("user");
        transcript.append("Early");
        let middle = transcript.begin("assistant");
        transcript.append("Unchanged middle");
        let tail = transcript.begin("user");
        transcript.append("Tail");
        assert!(middle > early && tail > middle);
        pending.changed(early);
        pending
            .flush(
                &store,
                id,
                &transcript,
                &Default::default(),
                SaveReason::Routine,
                now,
            )
            .unwrap();
        transcript.rows[early].text.push_str(" edited");
        transcript.rows[tail].text.push_str(" edited");
        pending.dirty_block(DirtyBlock {
            block_seq: 0,
            first_row: early as i64,
            last_row: early as i64,
        });
        pending.dirty_block(DirtyBlock {
            block_seq: 0,
            first_row: tail as i64,
            last_row: tail as i64,
        });
        assert!(pending.due(false, now + SHARED_SAVE_INTERVAL));
        pending
            .flush(
                &store,
                id,
                &transcript,
                &Default::default(),
                SaveReason::Routine,
                now + SHARED_SAVE_INTERVAL,
            )
            .unwrap();
        assert!(pending.from.is_none());
        assert!(pending.dirty.is_empty());
        assert_eq!(
            locked(&store).unwrap().load(id).unwrap().rows,
            transcript.rows
        );
        assert!(!pending.due(true, now + SHARED_SAVE_INTERVAL));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use monocode_core::follow_up::{Message, State};

    fn conversation() -> (Database, i64, Transcript) {
        let mut store = Store::open_monocode(std::path::Path::new(":memory:")).unwrap();
        let session = store.create("codex", "/project").unwrap();
        let mut transcript = Transcript::default();
        transcript.begin("user");
        transcript.append("Prompt");
        transcript.begin("assistant");
        store.write_rows(session.id, 0, &transcript.rows).unwrap();
        (Arc::new(Mutex::new(store)), session.id, transcript)
    }

    #[test]
    fn token_batches_coalesce_but_final_stop_flush_preserves_all_pending_rows() {
        let (store, id, mut transcript) = conversation();
        let now = Instant::now();
        let mut pending = PendingSave::new(true, now);
        for text in ["One ", "two ", "three"] {
            pending.changed(transcript.append(text));
        }
        assert!(!pending.due(false, now + Duration::from_millis(200)));
        assert!(!pending.due(false, now + Duration::from_millis(600)));
        assert!(pending.due(true, now + Duration::from_millis(200)));
        assert!(pending.due(false, now + SHARED_SAVE_INTERVAL));
        assert!(
            locked(&store).unwrap().load(id).unwrap().rows[1]
                .text
                .is_empty()
        );
        let stopped = transcript.begin("error");
        transcript.append("Stopped");
        pending.changed(stopped);
        pending
            .flush(
                &store,
                id,
                &transcript,
                &Default::default(),
                SaveReason::Final,
                now,
            )
            .unwrap();
        assert_eq!(
            locked(&store).unwrap().load(id).unwrap().rows,
            transcript.rows
        );
        assert!(!pending.due(true, now));
    }

    #[test]
    fn accepted_queue_message_flushes_prior_tokens_and_its_receipt_atomically() {
        let (store, id, mut transcript) = conversation();
        locked(&store)
            .unwrap()
            .save_follow_ups(
                id,
                &State {
                    revision: 1,
                    messages: vec![Message {
                        id: "queued".into(),
                        text: "Next".into(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        let now = Instant::now();
        let mut pending = PendingSave::new(true, now);
        pending.changed(transcript.append("Still streaming"));
        let from = transcript.begin("user");
        transcript.append("Next");
        pending
            .accept(&store, id, from, &transcript, Some("queued"), now)
            .unwrap();
        let store = locked(&store).unwrap();
        assert_eq!(store.load(id).unwrap().rows, transcript.rows);
        assert!(store.follow_ups(id).unwrap().messages.is_empty());
        assert_eq!(
            store
                .setting(&format!("mono-delivered:{id}:queued"))
                .unwrap(),
            Some("1".into())
        );
    }

    #[test]
    fn legacy_row_store_keeps_incremental_save_behavior() {
        let now = Instant::now();
        let mut pending = PendingSave::new(false, now);
        pending.changed(0);
        assert!(pending.due(false, now));
    }

    #[test]
    fn settled_turn_receipt_persists_without_a_view_and_keeps_newer_picker_settings() {
        let (store, id, transcript) = conversation();
        let mut settings = locked(&store).unwrap().agent_settings(id).unwrap().unwrap();
        settings.revision = 42;
        settings.model = Some("next-turn-model".into());
        locked(&store)
            .unwrap()
            .save_agent_settings(id, &settings)
            .unwrap();
        let started_at = transcript.rows[0].sent_at.unwrap();
        save_turn_receipt(
            &store,
            id,
            1,
            monocode_core::agent::TurnInfo {
                provider: "codex".into(),
                model: "Turn Model".into(),
                model_id: Some("turn-model".into()),
                provider_turn_id: Some("turn-id".into()),
                started_at: Some(started_at),
                duration_ms: 9000,
                duration_known: true,
                completed_at: Some(started_at + 9000),
                metrics: Some(monocode_core::agent::TurnMetrics {
                    output_tokens: 5,
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
        let store = locked(&store).unwrap();
        let saved = store.agent_settings(id).unwrap().unwrap();
        assert_eq!(saved.revision, 42);
        assert_eq!(saved.model.as_deref(), Some("next-turn-model"));
        assert_eq!(saved.turns[&1].model, "Turn Model");
        assert_eq!(saved.turns[&1].duration_ms, 9000);
        assert_eq!(saved.turns[&1].provider_turn_id.as_deref(), Some("turn-id"));
        assert_eq!(store.load(id).unwrap().rows, transcript.rows);
    }

    #[test]
    fn cancelled_initial_delivery_replays_accepted_rows_in_the_final_update() {
        let (store, id, history) = conversation();
        let mut summary = locked(&store).unwrap().sessions().unwrap().pop().unwrap();
        // Even a regression in cancellation must never launch a real provider.
        summary.provider = "invalid-test-provider".into();
        let (tx, rx) = async_channel::bounded(8);
        let (_control, controls) = std::sync::mpsc::channel();
        run_turn(
            summary,
            "Accepted before stopping".into(),
            Settings::default(),
            controls,
            store.clone(),
            Arc::new(AtomicBool::new(true)),
            tx,
        );
        let result = rx.try_recv().unwrap();
        assert!(result.finished);
        assert_eq!(result.status, "Ready");
        assert_eq!(result.start, history.rows.len());
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0].text, "Accepted before stopping");
        assert_eq!(result.rows[1].role, "assistant");
        let mut visible = history.rows;
        visible.extend(result.rows);
        assert_eq!(visible, locked(&store).unwrap().load(id).unwrap().rows);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn rejected_preflight_preserves_existing_history_without_a_provider_launch() {
        let (store, id, transcript) = conversation();
        let summary = locked(&store).unwrap().sessions().unwrap().pop().unwrap();
        let (tx, rx) = async_channel::bounded(2);
        let (_control, controls) = std::sync::mpsc::channel();
        run_turn(
            summary,
            "Unsent".into(),
            Settings {
                worktree_removed: true,
                mono_message_id: Some("not-accepted".into()),
                ..Default::default()
            },
            controls,
            store.clone(),
            Arc::new(AtomicBool::new(false)),
            tx,
        );
        let result = rx.try_recv().unwrap();
        assert!(result.finished);
        assert!(result.status.contains("worktree"));
        assert!(result.rows.is_empty());
        assert!(
            result.delivered.is_none(),
            "Rejected messages must stay queued"
        );
        assert_eq!(
            locked(&store).unwrap().load(id).unwrap().rows,
            transcript.rows
        );
    }

    #[test]
    fn worktree_creation_follows_message_acceptance_and_reports_the_checkout_in_the_final_update() {
        let root = std::env::temp_dir().join(format!(
            "monocode-optimistic-worktree-{}",
            monocode_core::mono::new_id()
        ));
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "-q"],
            vec![
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--allow-empty",
                "-qm",
                "Initial",
            ],
        ] {
            let result = std::process::Command::new("git")
                .current_dir(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let mut summary = store.create("codex", repo.to_str().unwrap()).unwrap();
        summary.provider = "invalid-test-provider".into();
        let id = summary.id;
        let store = Arc::new(Mutex::new(store));
        let (tx, rx) = async_channel::bounded(8);
        let (_control, controls) = std::sync::mpsc::channel();
        run_turn(
            summary,
            "Create the checkout".into(),
            Settings {
                new_worktree: true,
                ..Default::default()
            },
            controls,
            store.clone(),
            Arc::new(AtomicBool::new(false)),
            tx,
        );
        let first = rx.try_recv().unwrap();
        assert!(!first.finished);
        assert_eq!(first.status, "Creating worktree…");
        assert_eq!(first.rows[0].text, "Create the checkout");
        assert!(first.worktree_cwd.is_none());
        let created = rx.try_recv().unwrap();
        let cwd = created.worktree_cwd.unwrap();
        assert!(std::path::Path::new(&cwd).is_dir());
        let finished = rx.try_recv().unwrap();
        assert!(finished.finished);
        assert_eq!(finished.worktree_cwd.as_deref(), Some(cwd.as_str()));
        let store = locked(&store).unwrap();
        let settings = store.agent_settings(id).unwrap().unwrap();
        assert!(!settings.new_worktree);
        assert_eq!(settings.worktree_cwd.as_deref(), Some(cwd.as_str()));
        let rows = store.load(id).unwrap().rows;
        assert_eq!(
            rows.iter()
                .filter(|row| row.first && row.is_prompt())
                .count(),
            1
        );
        assert_eq!(rows[0].text, "Create the checkout");
        std::fs::remove_dir_all(root).unwrap();
    }
}
