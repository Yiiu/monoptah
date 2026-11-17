//! One fixed setting key, saved off the UI thread. The Inbox's ViewState is
//! the first consumer: the newest accepted value wins, a save never takes the
//! Store lock on the caller's thread, and quit drains the accepted value before
//! the process goes down. Values needing a transactional business receipt keep
//! their own paths; this saver only persists latest-value UI state.
use crate::runtime::{Database, locked};
use gpui::{App, BackgroundExecutor, Global};
use std::sync::{Arc, Mutex};

/// The setting key `ViewState` reads back on startup (core `inbox.rs`).
pub(crate) const INBOX_KEY: &str = "native-inbox-state";

/// The outcome of one accepted value, delivered once to its ticket.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SaveOutcome {
    Committed { generation: u64 },
    Superseded { by: u64 },
}

/// The receipt for one accepted value: its generation and where the single
/// outcome arrives. A dropped receiver cancels nothing; the save still runs.
pub(crate) struct SaveTicket {
    pub(crate) generation: u64,
    pub(crate) result: async_channel::Receiver<Result<SaveOutcome, String>>,
}

/// One value in flight. Each job's receipt sends its single outcome at the
/// point its fate is decided (a superseded pending value at acceptance, the
/// executing value at worker exit) and the job is then dropped, so no ticket
/// ever receives a second result; `receipt: None` marks a failed save's
/// re-armed value, whose outcome was already delivered, and `error` marks a
/// value a failed save left retryable.
struct Job {
    generation: u64,
    value: String,
    receipt: Option<async_channel::Sender<Result<SaveOutcome, String>>>,
    error: Option<String>,
}

/// The saver's whole state: the committed value, the value the worker is
/// writing, the latest accepted value waiting for it, and the generation
/// tickets are stamped with. At most one executing and one pending value.
#[derive(Default)]
struct Inner {
    committed: Option<String>,
    executing: Option<Job>,
    pending: Option<Job>,
    generation: u64,
    running: bool,
    closing: bool,
}

/// One ticket gets exactly one result, sent where its outcome is decided.
fn deliver(job: &Job, outcome: Result<SaveOutcome, String>) {
    if let Some(receipt) = &job.receipt {
        let _ = receipt.try_send(outcome);
    }
}

#[derive(Clone)]
pub(crate) struct UiStoreSaves {
    store: Database,
    key: &'static str,
    executor: BackgroundExecutor,
    state: Arc<Mutex<Inner>>,
    /// Wakes drain watchers; sends coalesce, and every worker exit is announced.
    progress: async_channel::Sender<()>,
    progress_rx: async_channel::Receiver<()>,
}

impl UiStoreSaves {
    pub(crate) fn new(store: Database, key: &'static str, executor: BackgroundExecutor) -> Self {
        let (progress, progress_rx) = async_channel::bounded(1);
        Self {
            store,
            key,
            executor,
            state: Arc::new(Mutex::new(Inner::default())),
            progress,
            progress_rx,
        }
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Accepts `value` as the latest wanted state for the fixed key and returns
    /// its ticket. Only the newest accepted value is ever written; an older
    /// waiting one is superseded. This never takes the Store lock, so the
    /// calling thread keeps running while another holder keeps the Store busy.
    /// An identical re-save of the committed value with nothing else in flight
    /// skips the Store entirely.
    pub(crate) fn save_latest(&self, value: String) -> Result<SaveTicket, String> {
        let (receipt, result) = async_channel::bounded(1);
        let mut state = self.inner();
        if state.closing {
            return Err("Not saved: the app is closing".into());
        }
        state.generation += 1;
        let generation = state.generation;
        if state.executing.is_none()
            && state.pending.is_none()
            && state.committed.as_deref() == Some(value.as_str())
        {
            let _ = receipt.try_send(Ok(SaveOutcome::Committed { generation }));
            return Ok(SaveTicket { generation, result });
        }
        if let Some(old) = state.pending.take() {
            deliver(&old, Ok(SaveOutcome::Superseded { by: generation }));
        }
        state.pending = Some(Job {
            generation,
            value,
            receipt: Some(receipt),
            error: None,
        });
        self.start_worker_if_idle(&mut state);
        Ok(SaveTicket { generation, result })
    }

    /// Stops accepting new values and waits, off the caller's thread, until
    /// every value accepted so far is committed. `Ok` means the Store holds the
    /// newest accepted value; `Err` leaves that value retryable and the saver
    /// accepting saves again, so a failed quit can simply be attempted again.
    pub(crate) fn drain(&self) -> async_channel::Receiver<Result<(), String>> {
        let (report, result) = async_channel::bounded(1);
        let mut state = self.inner();
        state.closing = true;
        self.start_worker_if_idle(&mut state);
        drop(state);
        let watcher = self.clone();
        self.executor
            .spawn(async move {
                loop {
                    let verdict = {
                        let mut state = watcher.inner();
                        if state.executing.is_some() || (state.pending.is_some() && state.running) {
                            None // A value is still being written.
                        } else if let Some(pending) = state.pending.as_ref() {
                            // The worker stopped on a failed save that nothing
                            // newer replaced: report it and take saves again.
                            let error = pending
                                .error
                                .clone()
                                .unwrap_or_else(|| "Saving the setting failed".into());
                            state.closing = false;
                            Some(Err(error))
                        } else {
                            Some(Ok(()))
                        }
                    };
                    match verdict {
                        Some(outcome) => {
                            let _ = report.try_send(outcome);
                            return;
                        }
                        None => {
                            let _ = watcher.progress_rx.recv().await;
                        }
                    }
                }
            })
            .detach();
        result
    }

    fn start_worker_if_idle(&self, state: &mut Inner) {
        if !state.running {
            state.running = true;
            let worker = self.clone();
            self.executor
                .spawn(async move { worker.work().await })
                .detach();
        }
    }

    /// Writes executing values until none are left. Runs on the background
    /// executor: the Store lock is taken here, never while the internal state
    /// lock is held, and never on the UI thread.
    async fn work(self) {
        loop {
            let value = {
                let mut state = self.inner();
                if state.executing.is_none()
                    && let Some(pending) = state.pending.take()
                {
                    state.executing = Some(pending);
                }
                match state.executing.as_ref() {
                    Some(job) => job.value.clone(),
                    None => {
                        state.running = false;
                        drop(state);
                        let _ = self.progress.try_send(());
                        return;
                    }
                }
            };
            let saved = locked(&self.store).and_then(|store| store.save_setting(self.key, &value));
            let mut state = self.inner();
            // Single worker: the executing slot still holds this job.
            let finished = state
                .executing
                .take()
                .expect("the worker owns the executing job");
            match saved {
                Ok(()) => {
                    state.committed = Some(finished.value.clone());
                    deliver(
                        &finished,
                        Ok(SaveOutcome::Committed {
                            generation: finished.generation,
                        }),
                    );
                }
                Err(error) => {
                    deliver(&finished, Err(error.clone()));
                    if state.pending.is_none() {
                        // Nothing newer was accepted: keep the failed value
                        // retryable instead of dropping it silently. A later
                        // save_latest or drain commits it.
                        state.pending = Some(Job {
                            generation: finished.generation,
                            value: finished.value.clone(),
                            receipt: None,
                            error: Some(error),
                        });
                        state.running = false;
                        drop(state);
                        let _ = self.progress.try_send(());
                        return;
                    }
                    // A newer value is already waiting for this worker.
                }
            }
            drop(state);
            let _ = self.progress.try_send(());
        }
    }
}

/// The app global holding the Inbox's saver, installed once at startup so every
/// Inbox view and the quit path share one fixed-key instance.
pub(crate) struct Shared(pub(crate) UiStoreSaves);
impl Global for Shared {}

pub(crate) fn install(store: Database, cx: &mut App) {
    if cx.try_global::<Shared>().is_some() {
        return;
    }
    let saver = UiStoreSaves::new(store, INBOX_KEY, cx.background_executor().clone());
    cx.set_global(Shared(saver));
}

pub(crate) fn inbox_saver(cx: &App) -> Option<UiStoreSaves> {
    cx.try_global::<Shared>().map(|shared| shared.0.clone())
}

/// Test seam: holds the Store mutex on a real thread, so anything needing the
/// Store cannot proceed while the test thread (the UI thread's stand-in) stays
/// free. `release` reports whether the caller's sentinel arrived while the lock
/// was still held.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::runtime::Database;
    use std::{sync::mpsc, thread, time::Duration};

    pub(crate) struct HeldStore {
        sentinel: mpsc::Sender<()>,
        settled: mpsc::Receiver<bool>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl HeldStore {
        pub(crate) fn hold(store: &Database) -> Self {
            let (held, held_rx) = mpsc::channel();
            let (sentinel, sentinel_rx) = mpsc::channel();
            let (settled_tx, settled_rx) = mpsc::channel();
            let store = store.clone();
            let thread = thread::spawn(move || {
                let _guard = store.lock().unwrap();
                held.send(()).unwrap();
                // The timeout only turns a regression (an inline lock on the
                // caller's thread) into a failed assertion instead of a hung
                // suite; no assertion waits on it.
                let kept = sentinel_rx.recv_timeout(Duration::from_secs(30)).is_ok();
                settled_tx.send(kept).unwrap();
            });
            held_rx.recv().unwrap();
            Self {
                sentinel,
                settled: settled_rx,
                thread: Some(thread),
            }
        }

        pub(crate) fn release(mut self) -> bool {
            let _ = self.sentinel.send(());
            let kept = self.settled.recv().unwrap_or(false);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            kept
        }
    }
}

#[cfg(test)]
mod persistence_safety_tests {
    use super::*;
    use gpui::TestAppContext;
    use monocode_core::store::Store;
    use std::{path::Path, sync::Arc};

    const KEY: &str = "native-inbox-state";

    fn memory_store() -> Database {
        Arc::new(Mutex::new(Store::open(Path::new(":memory:")).unwrap()))
    }

    fn saver(store: &Database, cx: &TestAppContext) -> UiStoreSaves {
        UiStoreSaves::new(store.clone(), KEY, cx.background_executor.clone())
    }

    fn committed(store: &Database) -> Option<String> {
        store.lock().unwrap().setting(KEY).unwrap()
    }

    fn fail_next_commit(store: &Database) {
        store
            .lock()
            .unwrap()
            .fail_next_save_commit_for_test()
            .unwrap();
    }

    #[gpui::test]
    fn a_b_c_coalesces_to_a_c(cx: &mut TestAppContext) {
        let store = memory_store();
        let saves = saver(&store, cx);
        let a = saves.save_latest("a".into()).unwrap();
        // Barrier: A finishes before B and C are accepted.
        cx.dispatcher.tick(true);
        assert!(matches!(
            a.result.try_recv(),
            Ok(Ok(SaveOutcome::Committed { .. }))
        ));
        assert_eq!(committed(&store).as_deref(), Some("a"));
        let held = test_support::HeldStore::hold(&store);
        let b = saves.save_latest("b".into()).unwrap();
        let c = saves.save_latest("c".into()).unwrap();
        assert!(matches!(
            b.result.try_recv(),
            Ok(Ok(SaveOutcome::Superseded { by })) if by == c.generation
        ));
        assert!(
            c.result.try_recv().is_err(),
            "C waits behind the held Store"
        );
        held.release();
        cx.run_until_parked();
        assert!(matches!(
            c.result.try_recv(),
            Ok(Ok(SaveOutcome::Committed { .. }))
        ));
        assert_eq!(committed(&store).as_deref(), Some("c"));
    }

    #[gpui::test]
    fn a_b_a_commits_a_last(cx: &mut TestAppContext) {
        let store = memory_store();
        let saves = saver(&store, cx);
        let first = saves.save_latest("a".into()).unwrap();
        cx.dispatcher.tick(true);
        assert!(matches!(
            first.result.try_recv(),
            Ok(Ok(SaveOutcome::Committed { .. }))
        ));
        assert_eq!(committed(&store).as_deref(), Some("a"));
        // B is accepted while its save cannot proceed (the Store is held), then
        // A is requested again: A must not be deduped against the in-flight B.
        let held = test_support::HeldStore::hold(&store);
        let b = saves.save_latest("b".into()).unwrap();
        let again = saves.save_latest("a".into()).unwrap();
        assert!(matches!(
            b.result.try_recv(),
            Ok(Ok(SaveOutcome::Superseded { by })) if by == again.generation
        ));
        assert!(
            again.result.try_recv().is_err(),
            "the identical value cannot skip past the in-flight B"
        );
        held.release();
        cx.run_until_parked();
        assert!(matches!(
            again.result.try_recv(),
            Ok(Ok(SaveOutcome::Committed { .. }))
        ));
        assert_eq!(committed(&store).as_deref(), Some("a"));
    }

    #[gpui::test]
    fn failed_identical_value_can_retry(cx: &mut TestAppContext) {
        let store = memory_store();
        fail_next_commit(&store);
        let saves = saver(&store, cx);
        let failed = saves.save_latest("same".into()).unwrap();
        cx.run_until_parked();
        assert!(
            matches!(failed.result.try_recv(), Ok(Err(_))),
            "the failing commit reports the error"
        );
        assert_eq!(committed(&store), None);
        // Retrying the identical value must not be deduped away.
        let retried = saves.save_latest("same".into()).unwrap();
        cx.run_until_parked();
        assert!(matches!(
            retried.result.try_recv(),
            Ok(Ok(SaveOutcome::Committed { .. }))
        ));
        assert_eq!(committed(&store).as_deref(), Some("same"));
    }

    #[gpui::test]
    fn idle_successful_duplicate_skips_store(cx: &mut TestAppContext) {
        let store = memory_store();
        let saves = saver(&store, cx);
        let first = saves.save_latest("a".into()).unwrap();
        cx.dispatcher.tick(true);
        assert!(matches!(
            first.result.try_recv(),
            Ok(Ok(SaveOutcome::Committed { .. }))
        ));
        let held = test_support::HeldStore::hold(&store);
        let duplicate = saves.save_latest("a".into()).unwrap();
        // Resolved without the Store: the ticket is committed while the Store
        // mutex is held elsewhere.
        assert!(matches!(
            duplicate.result.try_recv(),
            Ok(Ok(SaveOutcome::Committed { .. }))
        ));
        assert!(held.release());
        // A different value in flight removes the skip for the committed value.
        let held = test_support::HeldStore::hold(&store);
        let changing = saves.save_latest("b".into()).unwrap();
        let latest = saves.save_latest("a".into()).unwrap();
        assert!(matches!(
            changing.result.try_recv(),
            Ok(Ok(SaveOutcome::Superseded { by })) if by == latest.generation
        ));
        assert!(
            latest.result.try_recv().is_err(),
            "the identical value must wait behind the in-flight B"
        );
        held.release();
        cx.run_until_parked();
        assert!(matches!(
            latest.result.try_recv(),
            Ok(Ok(SaveOutcome::Committed { .. }))
        ));
        assert_eq!(committed(&store).as_deref(), Some("a"));
    }

    #[gpui::test]
    fn held_store_keeps_ui_running(cx: &mut TestAppContext) {
        let store = memory_store();
        let saves = saver(&store, cx);
        let held = test_support::HeldStore::hold(&store);
        // The save call is the UI thread's work: it must return, and the caller
        // must keep making progress, while the Store mutex is held elsewhere.
        let ticket = saves.save_latest("a".into()).unwrap();
        assert!(ticket.result.try_recv().is_err(), "nothing committed yet");
        assert!(
            held.release(),
            "the UI sentinel arrived while the Store mutex was still held"
        );
        cx.run_until_parked();
        assert!(matches!(
            ticket.result.try_recv(),
            Ok(Ok(SaveOutcome::Committed { .. }))
        ));
        assert_eq!(committed(&store).as_deref(), Some("a"));
    }

    #[gpui::test]
    fn drain_waits_for_latest_and_reports_failure(cx: &mut TestAppContext) {
        let store = memory_store();
        let saves = saver(&store, cx);
        let held = test_support::HeldStore::hold(&store);
        let latest = saves.save_latest("b".into()).unwrap();
        let drained = saves.drain();
        assert!(
            drained.try_recv().is_err(),
            "the drain waits for the latest value behind the held Store"
        );
        held.release();
        cx.run_until_parked();
        assert!(matches!(drained.try_recv(), Ok(Ok(()))));
        assert!(matches!(
            latest.result.try_recv(),
            Ok(Ok(SaveOutcome::Committed { .. }))
        ));
        assert_eq!(committed(&store).as_deref(), Some("b"));
        // A failing latest value fails the drain and leaves the saver usable.
        let store = memory_store();
        fail_next_commit(&store);
        let saves = saver(&store, cx);
        let failing = saves.save_latest("c".into()).unwrap();
        let failed = saves.drain();
        cx.run_until_parked();
        assert!(
            matches!(failing.result.try_recv(), Ok(Err(_))),
            "the failing value reports its error"
        );
        assert!(
            matches!(failed.try_recv(), Ok(Err(_))),
            "the failing drain reports the save error"
        );
        assert_eq!(committed(&store), None);
        // The saver accepts saves again, so quitting can be retried.
        let retried = saves.save_latest("d".into()).unwrap();
        cx.run_until_parked();
        assert!(matches!(
            retried.result.try_recv(),
            Ok(Ok(SaveOutcome::Committed { .. }))
        ));
        assert_eq!(committed(&store).as_deref(), Some("d"));
    }
}
