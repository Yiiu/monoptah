//! One activity cache for the sidebar, tabs, notifications and mobile views.
use super::*;
use monocode_core::{
    agent_status::{self, Activity, AgentStatus, Outcome, TurnEnd},
    store::SessionActivity,
    time::now_ms,
};

const PRESENCE_MS: u64 = 180_000;
static IDLE: AgentStatus = AgentStatus {
    activity: Activity::Idle,
    shells: Vec::new(),
};

#[derive(Default)]
pub(super) struct Timing {
    since: u64,
    active_turn: Option<u64>,
}

pub(super) enum Write {
    Begin(i64, u64),
    Finish(i64, u64, TurnEnd, String),
    Summary(i64, u64, String),
    ArchivedWorktree(i64, String),
    Seen(i64, u64, u64),
    Flush(async_channel::Sender<()>),
}

pub(super) fn start_writer(store: Database, cx: &App) -> async_channel::Sender<Write> {
    let (writes, pending) = async_channel::unbounded();
    cx.background_executor()
        .spawn(async move {
            while let Ok(write) = pending.recv().await {
                let save = || {
                    locked(&store).and_then(|store| {
                        let save = || match &write {
                            Write::Begin(id, at) => store.begin_session_turn(*id, *at),
                            Write::Finish(id, started, end, summary) => store
                                .finish_session_turn_with_summary(
                                    *id,
                                    *started,
                                    *end,
                                    Some(summary),
                                ),
                            Write::Summary(id, end, summary) => store
                                .update_progress_summary(*id, *end, summary)
                                .map(|_| ()),
                            Write::ArchivedWorktree(id, evidence) => {
                                store.remember_worktree_archive(*id, evidence)
                            }
                            Write::Seen(id, end, at) => store.see_session_turn(*id, *end, *at),
                            Write::Flush(done) => {
                                let _ = done.try_send(());
                                Ok(())
                            }
                        };
                        match save() {
                            Err(error)
                                if error.contains("no such table: session_activity")
                                    || error.contains("no such column")
                                    || error.contains("table session_activity has no column") =>
                            {
                                if store.repair_session_activity(now_ms())? {
                                    save()
                                } else {
                                    Err(error)
                                }
                            }
                            result => result,
                        }
                    })
                };
                let mut result = save();
                for delay in [50, 150, 350] {
                    if !result
                        .as_ref()
                        .is_err_and(|e| e.contains("locked") || e.contains("busy"))
                    {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(delay));
                    result = save();
                }
                if let Err(error) = result {
                    eprintln!("Cannot save agent session activity: {error}");
                }
            }
        })
        .detach();
    writes
}

pub(super) fn restored(saved: &HashMap<i64, SessionActivity>) -> HashMap<i64, AgentStatus> {
    saved
        .iter()
        .filter_map(|(&id, record)| {
            let last_turn = record.last_turn?;
            Some((
                id,
                agent_status::derive(&agent_status::StatusInput {
                    last_turn: Some(last_turn),
                    seen_at: record.seen_at,
                    ..Default::default()
                }),
            ))
        })
        .collect()
}

impl Shell {
    pub(crate) fn agent_status(&self, id: i64) -> &AgentStatus {
        self.agent_statuses.get(&id).unwrap_or(&IDLE)
    }

    pub(super) fn forget_agent_status(&mut self, id: i64) {
        self.agent_statuses.remove(&id);
        self.status_timing.remove(&id);
        self.session_activity.remove(&id);
        self.busy_sessions.remove(&id);
        self.progress_summary_jobs.remove(&id);
    }

    pub(super) fn has_running_status(&self) -> bool {
        self.agent_statuses.values().any(|status| {
            matches!(
                status.activity,
                Activity::Running { stale: false, .. }
                    | Activity::BackgroundWait {
                        reason: agent_status::Wait::UsageLimit { .. },
                        ..
                    }
            )
        })
    }

    pub(super) fn refresh_status_clocks(&mut self, cx: &mut Context<Self>) {
        let running: Vec<_> = self
            .agent_statuses
            .iter()
            .filter_map(|(&id, status)| {
                matches!(
                    status.activity,
                    Activity::Running { stale: false, .. }
                        | Activity::BackgroundWait {
                            reason: agent_status::Wait::UsageLimit { .. },
                            ..
                        }
                )
                .then_some(id)
            })
            .collect();
        for id in running {
            self.refresh_status(id, cx);
        }
    }

    pub(super) fn refresh_status(&mut self, pane: i64, cx: &mut Context<Self>) {
        let id = self.workspace.session_for_pane(pane);
        self.update_agent_status(id, None, cx);
    }

    pub(super) fn update_agent_status(
        &mut self,
        id: i64,
        external_outcome: Option<Outcome>,
        cx: &mut Context<Self>,
    ) {
        self.update_agent_status_for(id, None, external_outcome, cx);
    }

    pub(super) fn update_agent_status_for(
        &mut self,
        id: i64,
        finished_summary: Option<&Session>,
        external_outcome: Option<Outcome>,
        cx: &mut Context<Self>,
    ) {
        let view = self.notification_chat(id, cx);
        let chat = view.as_ref().map(|view| view.read(cx));
        let Some(summary) = chat
            .map(|chat| &chat.summary)
            .or_else(|| self.session_summaries.get(&id))
            .or(finished_summary)
        else {
            return;
        };
        let provider = summary.provider.clone();
        let now = now_ms();
        let (remote_running, remote_waiting) =
            self.remote_marks.get(&id).copied().unwrap_or_default();
        let terminal_running = self.terminal_busy(id, cx);
        let mut input = chat.map_or_else(
            || agent_status::StatusInput {
                now,
                ..Default::default()
            },
            |chat| chat.agent_status_input(now, true, cx),
        );
        input.remote_waiting = remote_waiting;
        input.streaming |= remote_running || terminal_running;
        let active = input.streaming || input.remote_waiting || input.wake_tasks > 0;
        let local = chat.is_some_and(|chat| !chat.is_remote()) || terminal_running;
        let outcome = external_outcome.or_else(|| chat.and_then(|chat| chat.agent_outcome));
        let mut finished = None;
        let mut summary_request = None;
        let waiting_on_limit = input.usage_limit_resume_at.is_some() || input.usage_limit_pending;
        let old_animated = self.agent_status(id).activity.is_animating();
        let was_waiting_on_limit = matches!(
            self.agent_status(id).activity,
            Activity::BackgroundWait {
                reason: agent_status::Wait::UsageLimit { .. }
                    | agent_status::Wait::UsageLimitPending,
                ..
            }
        );
        let old_activity = std::mem::discriminant(&self.agent_status(id).activity);
        let timing = self.status_timing.entry(id).or_insert_with(|| Timing {
            since: now,
            active_turn: None,
        });
        let record = self.session_activity.entry(id).or_default();

        if active {
            if timing.active_turn.is_none() {
                let at = record
                    .last_turn
                    .map_or(now, |end| now.max(end.at.saturating_add(1)));
                timing.active_turn = Some(at);
                self.progress_summary_jobs.remove(&id);
                if local {
                    let _ = self.activity_writes.try_send(Write::Begin(id, at));
                }
            }
        } else if let Some(started_at) = timing.active_turn.take() {
            let end = TurnEnd {
                at: now.max(started_at),
                outcome: outcome.unwrap_or(Outcome::Completed),
            };
            let (summary, request) = progress_summary::completed_progress(chat, end.outcome, cx);
            record.progress_summary = Some(summary.clone());
            record.last_turn = Some(end);
            let _ = self
                .activity_writes
                .try_send(Write::Finish(id, started_at, end, summary));
            summary_request = request;
            finished = Some(end);
        }

        let unseen_end = record
            .last_turn
            .filter(|end| record.seen_at.is_none_or(|seen_at| seen_at < end.at));
        if !active
            && !waiting_on_limit
            && let Some(end) = unseen_end
            && self.window_active
            && self
                .last_status_input_at
                .is_some_and(|at| now.saturating_sub(at) < PRESENCE_MS)
        {
            // If this pane is visible, acknowledge it after updating the cached receipt.
            // That also covers the Mono panel without reading its entity during installation.
            let shell = cx.weak_entity();
            cx.defer(move |cx| {
                let _ = shell.update(cx, |this, cx| {
                    if this
                        .session_activity
                        .get(&id)
                        .and_then(|record| record.last_turn)
                        == Some(end)
                        && this.status_user_present(now_ms())
                        && this.notice_session_visible(id, cx)
                    {
                        this.mark_agent_seen(id, cx);
                    }
                });
            });
        }

        input.last_turn = record.last_turn;
        input.seen_at = record.seen_at;
        input.state_since = timing.since;
        input.last_event_at = input
            .last_event_at
            .max(timing.active_turn.unwrap_or_default());
        if matches!(
            self.agent_statuses.get(&id).map(|status| &status.activity),
            Some(Activity::Running { .. })
        ) {
            input.last_event_at = input.last_event_at.max(timing.since);
        }
        let mut status = agent_status::derive(&input);
        if old_activity != std::mem::discriminant(&status.activity) {
            timing.since = now;
            match &mut status.activity {
                Activity::Running { since, stale, .. } => {
                    *since = now;
                    *stale = false;
                }
                Activity::BackgroundWait { since, .. } => {
                    *since = now;
                }
                _ => {}
            }
        }
        let mark = MarkState::from_activity(&status.activity);
        let busy_changed = if status.is_busy() {
            self.busy_sessions.insert(id)
        } else {
            self.busy_sessions.remove(&id)
        };
        for entity in [self.session_marks.get(&id), self.tab_marks.get(&id)]
            .into_iter()
            .flatten()
        {
            entity.update(cx, |entity, cx| entity.set(&provider, mark, cx));
        }
        let changed = self.agent_status(id) != &status;
        let animation_changed = old_animated != status.activity.is_animating();
        let finished_outcome = match status.activity {
            Activity::Done { outcome, .. } if finished.is_some() || was_waiting_on_limit => {
                Some(outcome)
            }
            _ => None,
        };
        if let Some(view) = &view
            && view.read(cx).displayed_activity != status.activity
        {
            let activity = status.activity.clone();
            view.update(cx, |chat, cx| {
                chat.displayed_activity = activity;
                cx.notify();
            });
        }
        let was_unread = self.agent_status(id).unread_done();
        self.tree_note_unread(id, was_unread, &status.activity, cx);
        self.agent_statuses.insert(id, status);
        self.sync_agent_status_tick(cx);
        if busy_changed || animation_changed {
            self.push_inbox_busy(cx);
        }
        if changed {
            self.sync_today_sessions(cx);
            self.schedule_mobile_push(cx);
            self.invalidate_chrome(cx);
            cx.notify();
        }
        if let Some(outcome) = finished_outcome
            && let Some(view) = view
        {
            self.notification_turn_finished(&view, outcome, cx);
        }
        if let Some(end) = finished
            && let Some(request) = summary_request
        {
            self.queue_progress_summary(id, end.at, request, cx);
        }
    }

    fn status_user_present(&self, now: u64) -> bool {
        self.window_active
            && self
                .last_status_input_at
                .is_some_and(|at| now.saturating_sub(at) < PRESENCE_MS)
    }

    pub(super) fn note_status_input(&mut self, cx: &mut Context<Self>) {
        let now = now_ms();
        let was_present = self.status_user_present(now);
        self.last_status_input_at = Some(now);
        if !was_present {
            self.refresh_visible_seen(cx);
        }
    }

    pub(super) fn refresh_visible_seen(&mut self, cx: &mut Context<Self>) {
        if !self.status_user_present(now_ms()) {
            return;
        }
        let visible: Vec<_> = self
            .agent_statuses
            .iter()
            .filter_map(|(&id, status)| {
                (status.unread_done() && self.notice_session_visible(id, cx)).then_some(id)
            })
            .collect();
        for id in visible {
            self.mark_agent_seen(id, cx);
        }
    }

    pub(super) fn mark_agent_seen(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(record) = self.session_activity.get_mut(&id) else {
            return;
        };
        let Some(end) = record.last_turn else { return };
        if record.seen_at.is_some_and(|at| at >= end.at) {
            return;
        }
        record.seen_at = Some(end.at);
        let _ = self
            .activity_writes
            .try_send(Write::Seen(id, end.at, now_ms().max(end.at)));
        self.update_agent_status(id, None, cx);
    }
}
