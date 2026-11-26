//! SessionFiltersMenu.tsx: persisted status, time and provider checkboxes.
use super::*;
use monocode_core::session_filters::{Status, TimeFilter};

impl Shell {
    pub(super) fn filter_menu_items(&self, cx: &App) -> Vec<MenuItem> {
        let filters = &self.workspace.session_filters;
        let mut items = vec![
            MenuAction::new("archived", "Archived")
                .checked(self.workspace.preferences.show_archived)
                .into(),
            MenuItem::Header("Status".into()),
        ];
        for (id, label, checked) in [
            ("working", "Working", filters.status.working),
            ("approval", "Needs approval", filters.status.needs_approval),
            ("done", "Done", filters.status.done),
        ] {
            items.push(
                MenuAction::new(id, label)
                    .checked(checked)
                    .keep_open()
                    .into(),
            );
        }
        items.push(MenuItem::Header("Time".into()));
        for (time, id, label) in TimeFilter::OPTIONS {
            items.push(
                MenuAction::new(format!("time:{id}"), label)
                    .checked(filters.time == time)
                    .keep_open()
                    .into(),
            );
        }
        let providers: Vec<_> = Kind::ALL
            .into_iter()
            .filter(|kind| {
                self.order.iter().any(|id| {
                    let session = self.session_metadata(*id, cx);
                    self.project_of(*id, &session.summary.cwd) == self.workspace.project
                        && session.summary.has_messages
                        && session.summary.provider == kind.id()
                        && self.in_sidebar_worktree_scope(*id, cx)
                })
            })
            .collect();
        if !providers.is_empty() {
            items.push(MenuItem::Header("Provider".into()));
            for kind in providers {
                items.push(
                    MenuAction::new(format!("provider:{}", kind.id()), kind.label())
                        .image(kind.id())
                        .checked(!filters.hidden_providers.iter().any(|id| id == kind.id()))
                        .keep_open()
                        .into(),
                );
            }
        }
        if filters.active(self.workspace.preferences.show_archived) {
            items.push(MenuItem::Separator);
            items.push(MenuAction::new("clear", "Clear filters").keep_open().into());
        }
        items
    }

    pub(super) fn change_session_filter(&mut self, action: &str, cx: &mut Context<Self>) {
        let filters = &mut self.workspace.session_filters;
        match action {
            "archived" => {
                self.workspace.preferences.show_archived = !self.workspace.preferences.show_archived
            }
            "working" => filters.status.working = !filters.status.working,
            "approval" => filters.status.needs_approval = !filters.status.needs_approval,
            "done" => filters.status.done = !filters.status.done,
            "clear" => {
                *filters = Default::default();
                self.workspace.preferences.show_archived = false;
            }
            _ => {
                if let Some(provider) = action.strip_prefix("provider:") {
                    filters.toggle_provider(provider);
                } else if let Some(time) = action.strip_prefix("time:").and_then(|id| {
                    TimeFilter::OPTIONS
                        .iter()
                        .find(|(_, value, _)| *value == id)
                        .map(|(t, _, _)| *t)
                }) {
                    filters.time = time;
                } else {
                    return;
                }
            }
        }
        self.close_preview(cx);
        let visible = if self.workspace.tree_layout() {
            self.tree.archived_open = None;
            self.tree_session_ids(cx)
        } else {
            self.sidebar_session_ids(cx)
        };
        self.selected_sessions.retain(|id| visible.contains(id));
        self.selection_anchor = self.selection_anchor.filter(|id| visible.contains(id));
        self.invalidate_chrome(cx);
        self.persist(cx);
        cx.notify();
    }

    pub(super) fn filter_session(&self, id: i64, start: u64, cx: &App) -> bool {
        let session = self.session_metadata(id, cx);
        let status = self.agent_status(id);
        self.workspace.session_filters.matches(
            session.summary,
            Status {
                working: status.is_busy(),
                needs_approval: status.needs_you(),
                done: status.unread_done(),
            },
            start,
            self.workspace.preferences.show_archived,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use monocode_core::agent_status::{Activity, AgentStatus, Outcome, Wait};

    #[gpui::test]
    fn working_filter_excludes_a_remote_session_that_needs_input(cx: &mut TestAppContext) {
        let (window, id, _) = super::super::tests::window(cx);
        window
            .update(cx, |shell, _, cx| {
                shell.remote_marks.insert(id, (true, true));
                shell.refresh_status(id, cx);
                assert!(shell.session_metadata(id, cx).busy);

                shell.workspace.session_filters.status.working = true;
                assert!(!shell.filter_session(id, 0, cx));

                shell.workspace.session_filters.status.working = false;
                shell.workspace.session_filters.status.needs_approval = true;
                assert!(shell.filter_session(id, 0, cx));
            })
            .unwrap();
    }

    #[gpui::test]
    fn filters_persist_and_update_without_closing_the_menu_or_active_chat(cx: &mut TestAppContext) {
        cx.update(|cx| {
            super::super::bind_keys(cx);
            crate::popover::bind(cx);
        });
        let (window, a, b) = super::super::tests::window(cx);
        window
            .update(cx, |shell, w, cx| {
                let project = shell.workspace.project.clone();
                for id in [a, b] {
                    shell.sessions[&id].update(cx, |s, _| {
                        s.summary.cwd = project.clone();
                        s.summary.has_messages = true;
                        s.summary.updated_at = monocode_core::time::now_ms();
                    });
                }
                shell.sessions[&a].update(cx, |s, _| {
                    s.summary.provider = "codex".into();
                    s.busy = true;
                });
                shell.refresh_status(a, cx);
                shell.sessions[&b].update(cx, |s, _| s.summary.provider = "claude".into());
                shell.selected_sessions.insert(b);
                shell.open_menu(
                    MenuTarget::SessionFilter,
                    Anchor::Point(gpui::point(px(220.), px(180.))),
                    w,
                    cx,
                );
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let row = visual.debug_bounds("menu-action-working").unwrap();
        visual.simulate_click(row.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                assert!(shell.menu.is_some(), "status checkboxes keep the menu open");
                assert_eq!(shell.sidebar_session_ids(cx), [a]);
                assert_eq!(shell.workspace.active, Some(b));
                assert!(shell.sessions[&a].read(cx).busy);
                assert!(!shell.selected_sessions.contains(&b));
                let state: Workspace =
                    serde_json::from_str(&shell.latest_state.lock().unwrap()).unwrap();
                assert!(state.session_filters.status.working);
            })
            .unwrap();
        let row = visual.debug_bounds("menu-action-clear").unwrap();
        visual.simulate_click(row.center(), Default::default());
        visual.run_until_parked();
        window
            .update(visual, |shell, _, cx| {
                assert_eq!(shell.sidebar_session_ids(cx).len(), 2);
                shell.sessions[&a].update(cx, |s, _| s.busy = false);
                shell.agent_statuses.insert(
                    a,
                    AgentStatus {
                        activity: Activity::Done {
                            outcome: Outcome::Completed,
                            at: monocode_core::time::now_ms(),
                            unread: true,
                        },
                        shells: Vec::new(),
                    },
                );
                shell.change_session_filter("done", cx);
                assert_eq!(shell.sidebar_session_ids(cx), [a]);
                shell.agent_statuses.insert(
                    b,
                    AgentStatus {
                        activity: Activity::BackgroundWait {
                            reason: Wait::Subtasks(2),
                            since: monocode_core::time::now_ms(),
                        },
                        shells: Vec::new(),
                    },
                );
                shell.workspace.session_filters.status.working = true;
                assert_eq!(shell.sidebar_session_ids(cx).len(), 2);
                shell.change_session_filter("provider:claude", cx);
                assert_eq!(shell.sidebar_session_ids(cx), [a]);
                shell.change_session_filter("time:today", cx);
                assert_eq!(shell.sidebar_session_ids(cx), [a]);
                shell.sessions[&a].update(cx, |s, _| s.summary.updated_at = 1);
                assert!(shell.sidebar_session_ids(cx).is_empty());
            })
            .unwrap();
    }
}
