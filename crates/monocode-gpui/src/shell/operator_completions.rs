//! Durable completion groups for ordinary project sessions.
use super::*;
use monocode_core::store::{OperatorCompletionGroup, OperatorSubscription, WorkerOutcome};
use std::collections::VecDeque;

type Deferred = Box<dyn FnOnce(&mut Shell, &mut Window, &mut Context<Shell>)>;

#[derive(Default)]
pub(super) struct Writes {
    active: HashSet<i64>,
    waiting: HashMap<i64, VecDeque<Deferred>>,
}

impl Shell {
    fn operator_worker_in_scope(&self, caller: i64, worker: i64, cx: &App) -> bool {
        let Some(parent) = self.session_summaries.get(&caller) else {
            return false;
        };
        self.session_summaries.get(&worker).is_some_and(|child| {
            !child.cwd.starts_with("remote://")
                && !self.is_cloud(worker)
                && monocode_core::files::path_key(&child.cwd)
                    == monocode_core::files::path_key(&parent.cwd)
                && caller != worker
                && !self.orchestration_worker(worker)
                && !self.is_terminal(worker)
                && self
                    .monos
                    .as_ref()
                    .is_none_or(|monos| !monos.read(cx).is_mono_session(worker))
        })
    }

    pub(super) fn observe_operator_completions(
        &mut self,
        view: &Entity<Conversation>,
        caller: i64,
        cx: &mut Context<Self>,
    ) {
        let input = view.read(cx).input.clone();
        let mut empty = input.read(cx).content.is_empty();
        self.observations
            .push(cx.observe(&input, move |this, input, cx| {
                let was_empty = empty;
                empty = input.read(cx).content.is_empty();
                if empty && !was_empty && this.operator_subscriptions.contains_key(&caller) {
                    this.check_operator_completions(cx);
                }
            }));
    }
    fn armed_operator_subscriptions(
        &self,
        caller: i64,
        group: OperatorCompletionGroup,
        session: i64,
        title: String,
    ) -> Vec<OperatorSubscription> {
        let mut subscriptions = self
            .operator_subscriptions
            .get(&caller)
            .cloned()
            .unwrap_or_default();
        subscriptions.retain(|s| s.session != session && !s.delivered);
        subscriptions.push(OperatorSubscription {
            session,
            title,
            group,
            outcome: None,
            delivered: false,
        });
        subscriptions
    }

    #[cfg(test)]
    pub(super) fn arm_operator_completion(
        &mut self,
        caller: i64,
        group: OperatorCompletionGroup,
        session: i64,
        title: String,
    ) -> Result<(), String> {
        let subscriptions = self.armed_operator_subscriptions(caller, group, session, title);
        self.operator_completion_storage_ready()?;
        locked(&self.store)?.save_operator_subscriptions(caller, &subscriptions)?;
        self.install_operator_completions(caller, subscriptions);
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn dismiss_operator_completion(
        &mut self,
        caller: i64,
        session: i64,
    ) -> Result<(), String> {
        self.operator_completion_storage_ready()?;
        let Some(mut subscriptions) = self.operator_subscriptions.get(&caller).cloned() else {
            return Ok(());
        };
        subscriptions.retain(|s| s.session != session);
        locked(&self.store)?.save_operator_subscriptions(caller, &subscriptions)?;
        self.install_operator_completions(caller, subscriptions);
        Ok(())
    }
    fn operator_completion_storage_ready(&self) -> Result<(), String> {
        match &self.operator_subscription_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
    fn install_operator_completions(
        &mut self,
        caller: i64,
        subscriptions: Vec<OperatorSubscription>,
    ) {
        if subscriptions.is_empty() {
            self.operator_subscriptions.remove(&caller);
        } else {
            self.operator_subscriptions.insert(caller, subscriptions);
        }
    }

    fn after_operator_write(
        &mut self,
        caller: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if self.operator_writes.active.contains(&caller) {
            self.operator_writes
                .waiting
                .entry(caller)
                .or_default()
                .push_back(Box::new(action));
        } else {
            action(self, window, cx);
        }
    }

    pub(super) fn save_operator_completions(
        &mut self,
        caller: i64,
        subscriptions: Vec<OperatorSubscription>,
        cx: &mut Context<Self>,
        after: impl FnOnce(&mut Self, Result<(), String>, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let started = self.operator_writes.active.insert(caller);
        debug_assert!(started);
        let store = self.store.clone();
        let storage = self.operator_completion_storage_ready();
        let saved = cx.background_executor().spawn(async move {
            let result = storage
                .and_then(|()| locked(&store)?.save_operator_subscriptions(caller, &subscriptions));
            (subscriptions, result)
        });
        cx.spawn(async move |this, cx| {
            let (subscriptions, result) = saved.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.operator_writes.active.remove(&caller);
                if result.is_ok() {
                    this.install_operator_completions(caller, subscriptions);
                }
                let saved = result.is_ok();
                after(this, result, window, cx);
                while !this.operator_writes.active.contains(&caller) {
                    let next = this
                        .operator_writes
                        .waiting
                        .get_mut(&caller)
                        .and_then(VecDeque::pop_front);
                    let Some(next) = next else {
                        this.operator_writes.waiting.remove(&caller);
                        break;
                    };
                    next(this, window, cx);
                }
                if saved {
                    this.check_operator_completions(cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn register_operator_completion(
        &mut self,
        caller: i64,
        group: OperatorCompletionGroup,
        child: &Session,
        window: &mut Window,
        cx: &mut Context<Self>,
        after: impl FnOnce(
            &mut Self,
            Result<Vec<OperatorSubscription>, String>,
            &mut Window,
            &mut Context<Self>,
        ) + 'static,
    ) {
        let (session, title) = (child.id, child.title.clone());
        self.after_operator_write(caller, window, cx, move |this, window, cx| {
            if this
                .session_summaries
                .get(&caller)
                .is_none_or(|session| session.archived)
                || this.deleting.contains(&caller)
                || this.archiving.contains(&caller)
            {
                after(
                    this,
                    Err("The Operator session is no longer available".into()),
                    window,
                    cx,
                );
                return;
            }
            let previous = this
                .operator_subscriptions
                .get(&caller)
                .cloned()
                .unwrap_or_default();
            let subscriptions = this.armed_operator_subscriptions(caller, group, session, title);
            this.save_operator_completions(
                caller,
                subscriptions,
                cx,
                move |this, result, window, cx| {
                    after(this, result.map(|()| previous), window, cx);
                },
            );
        });
    }

    pub(super) fn has_operator_completion(&self, caller: i64, session: i64) -> bool {
        self.operator_writes.active.contains(&caller)
            || self
                .operator_subscriptions
                .get(&caller)
                .is_some_and(|subscriptions| subscriptions.iter().any(|sub| sub.session == session))
    }

    pub(super) fn cancel_operator_completion(
        &mut self,
        caller: i64,
        session: i64,
        window: &mut Window,
        cx: &mut Context<Self>,
        after: impl FnOnce(&mut Self, Result<(), String>, &mut Window, &mut Context<Self>) + 'static,
    ) {
        self.after_operator_write(caller, window, cx, move |this, window, cx| {
            let Some(mut subscriptions) = this.operator_subscriptions.get(&caller).cloned() else {
                after(this, Ok(()), window, cx);
                return;
            };
            let size = subscriptions.len();
            subscriptions.retain(|s| s.session != session);
            if subscriptions.len() == size {
                after(this, Ok(()), window, cx);
            } else {
                this.save_operator_completions(caller, subscriptions, cx, after);
            }
        });
    }
    pub(super) fn restore_operator_completions(&mut self, cx: &mut Context<Self>) {
        // An unfinished provider turn cannot survive an app restart.
        for subscriptions in self.operator_subscriptions.values_mut() {
            for subscription in subscriptions.iter_mut().filter(|s| !s.delivered) {
                subscription.outcome.get_or_insert(WorkerOutcome::Stopped);
            }
        }
        self.check_operator_completions(cx);
    }
    fn operator_group_ready(&self, caller: i64, group: &OperatorCompletionGroup, cx: &App) -> bool {
        let Some(subscriptions) = self.operator_subscriptions.get(&caller) else {
            return false;
        };
        let group: Vec<_> = subscriptions.iter().filter(|s| &s.group == group).collect();
        !group.is_empty()
            && group.iter().all(|s| {
                !s.delivered
                    && s.outcome.is_some()
                    && (s.outcome == Some(WorkerOutcome::Removed)
                        || self.operator_worker_in_scope(caller, s.session, cx))
                    && self.session_summaries.get(&s.session).is_none_or(|_| {
                        let metadata = self.session_metadata(s.session, cx);
                        !metadata.busy
                    })
            })
    }
    pub(super) fn check_operator_completions(&mut self, cx: &mut Context<Self>) {
        let callers: Vec<_> = self.operator_subscriptions.keys().copied().collect();
        for caller in callers {
            if self.operator_writes.active.contains(&caller) {
                continue;
            }
            let mut subscriptions = self.operator_subscriptions[&caller].clone();
            if self
                .session_summaries
                .get(&caller)
                .is_none_or(|s| s.archived)
            {
                self.save_operator_completions(caller, Vec::new(), cx, show_save_error);
                continue;
            }
            if self.deleting.contains(&caller) {
                continue;
            }
            let mut changed = false;
            for subscription in subscriptions.iter_mut().filter(|s| !s.delivered) {
                let outcome = if !self.operator_worker_in_scope(caller, subscription.session, cx) {
                    Some(WorkerOutcome::Removed)
                } else if let Some(view) = self.sessions.get(&subscription.session) {
                    let view = view.read(cx);
                    if view.busy {
                        None
                    } else if subscription.outcome.is_none() {
                        view.worker_outcome()
                    } else {
                        subscription.outcome
                    }
                } else {
                    subscription.outcome
                };
                if subscription.outcome != outcome {
                    subscription.outcome = outcome;
                    changed = true;
                }
            }
            if changed {
                self.save_operator_completions(caller, subscriptions, cx, show_save_error);
                continue;
            }
            let groups: HashSet<_> = self.operator_subscriptions[&caller]
                .iter()
                .filter(|s| !s.delivered)
                .map(|s| s.group.clone())
                .collect();
            for group in groups {
                if self
                    .operator_completion_work
                    .contains(&(caller, group.clone()))
                    || self.archiving.contains(&caller)
                    || !self.operator_group_ready(caller, &group, cx)
                {
                    continue;
                }
                let Some(origin) = self.ensure_session(caller, cx) else {
                    continue;
                };
                origin.update(cx, |v, cx| v.load(cx));
                if !origin.read(cx).ready_for_coordinator_report(cx) {
                    continue;
                }
                let children: Vec<_> = self.operator_subscriptions[&caller]
                    .iter()
                    .filter(|s| s.group == group)
                    .cloned()
                    .collect();
                let responses: Vec<_> = children
                    .iter()
                    .map(|s| {
                        if s.outcome == Some(WorkerOutcome::Removed) {
                            return None;
                        }
                        self.sessions
                            .get(&s.session)
                            .filter(|v| v.read(cx).history_loaded())
                            .map(|v| v.read(cx).external_outcome(cx).1)
                    })
                    .collect();
                self.operator_completion_work
                    .insert((caller, group.clone()));
                let store = self.store.clone();
                let work = cx.background_executor().spawn(async move {
                    children
                        .iter()
                        .zip(responses)
                        .map(|(s, response)| {
                            let prose = match response {
                                Some(response) => response,
                                None if s.outcome == Some(WorkerOutcome::Removed) => String::new(),
                                None => {
                                    let transcript = locked(&store)?.load(s.session)?;
                                    monocode_core::response::final_prose(&transcript.rows)
                                }
                            };
                            Ok(format!(
                                "Session {}: {}\nOutcome: {}\n{}",
                                s.session,
                                s.title,
                                s.outcome.unwrap().label(),
                                monocode_core::response::limit(&prose, 12_000)
                            ))
                        })
                        .collect::<Result<Vec<_>, String>>()
                        .map(|blocks| (children, blocks.join("\n\n")))
                });
                cx.spawn(async move |this, cx| {
                    let report = work.await;
                    let _ = this.update(cx, |this, cx| {
                        this.deliver_operator_completion(caller, group, report, cx);
                    });
                })
                .detach();
            }
        }
    }
    pub(super) fn deliver_operator_completion(
        &mut self,
        caller: i64,
        group: OperatorCompletionGroup,
        report: Result<(Vec<OperatorSubscription>, String), String>,
        cx: &mut Context<Self>,
    ) {
        self.operator_completion_work
            .remove(&(caller, group.clone()));
        let (children, mut report) = match report {
            Ok(report) => report,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        if self
            .session_summaries
            .get(&caller)
            .is_none_or(|s| s.archived)
            || self.deleting.contains(&caller)
            || self.archiving.contains(&caller)
            || self.operator_writes.active.contains(&caller)
            || !self.operator_group_ready(caller, &group, cx)
        {
            return;
        }
        let current: Vec<_> = self.operator_subscriptions[&caller]
            .iter()
            .filter(|s| s.group == group)
            .map(|s| (s.session, s.outcome))
            .collect();
        if current
            != children
                .iter()
                .map(|s| (s.session, s.outcome))
                .collect::<Vec<_>>()
        {
            self.check_operator_completions(cx);
            return;
        }
        let Some(origin) = self.sessions.get(&caller).cloned() else {
            return;
        };
        if !origin.read(cx).ready_for_coordinator_report(cx) {
            return;
        }
        let previous = self.operator_subscriptions[&caller].clone();
        let mut subscriptions = previous.clone();
        for subscription in subscriptions.iter_mut().filter(|s| s.group == group) {
            subscription.delivered = true;
        }
        let pending: Vec<_> = subscriptions
            .iter()
            .filter(|s| !s.delivered)
            .map(|s| format!("Session {}: {}", s.session, s.title))
            .collect();
        if !pending.is_empty() {
            report.push_str(&format!(
                "\n\nOther reports pending:\n{}\nMore reports will follow.",
                pending.join("\n")
            ));
        }
        // Persist before dispatch for at-most-once delivery across a crash.
        self.save_operator_completions(caller, subscriptions, cx, move |this, saved, window, cx| {
            if saved.is_err() {
                show_save_error(this, saved, window, cx);
                return;
            }
            if this.session_summaries.get(&caller).is_none_or(|session| session.archived)
                || this.deleting.contains(&caller)
                || this.archiving.contains(&caller)
                || !origin.read(cx).ready_for_coordinator_report(cx)
            {
                // The parent changed while SQLite was writing. It has not
                // received the report, so restore the pending delivery.
                    let retry = previous.clone();
                    this.save_operator_completions(caller, previous, cx, move |this, result, window, cx| {
                        if result.is_err() {
                            this.install_operator_completions(caller, retry);
                        }
                        show_save_error(this, result, window, cx);
                    });
                return;
            }
            origin.update(cx, |view, cx| view.send_coordinator_report(format!(
                "[Delegated work completed]\nWorker completion is not verification: inspect the results, verify as needed, then tell the user.\n\n{report}"
            ), cx));
        });
    }
}

fn show_save_error(
    shell: &mut Shell,
    result: Result<(), String>,
    _: &mut Window,
    cx: &mut Context<Shell>,
) {
    if let Err(error) = result {
        shell.error = Some(error);
        cx.notify();
    }
}
