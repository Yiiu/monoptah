//! LiveAgentsPreview.tsx: the sidebar's card of working and unseen finished
//! sessions, across every project.
use super::*;
use monocode_core::agent_status::Activity;

const LIVE_AGENT_MIN: usize = 2;
const LIVE_AGENT_CAP: usize = 4;
const STATUS_TICK: std::time::Duration = std::time::Duration::from_secs(15);

struct LiveAgent {
    id: i64,
    cwd: String,
    provider: String,
    title: String,
    label: String,
    activity: Activity,
    duration_ms: Option<u64>,
}

/// liveAgents.ts's formatLiveElapsed.
pub(crate) fn live_elapsed(ms: u64) -> String {
    let seconds = ((ms + 500) / 1000).max(1);
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let (minutes, rest) = (seconds / 60, seconds % 60);
    if minutes < 60 {
        return if rest > 0 {
            format!("{minutes}m {rest}s")
        } else {
            format!("{minutes}m")
        };
    }
    let (hours, rest) = (minutes / 60, minutes % 60);
    if rest > 0 {
        format!("{hours}h {rest}m")
    } else {
        format!("{hours}h")
    }
}

impl Shell {
    fn live_agents(&self, now: u64, cx: &App) -> Vec<LiveAgent> {
        let mut agents: Vec<_> = self
            .order
            .iter()
            .filter_map(|id| {
                let status = self.agent_status(*id);
                let label = status.label(now)?;
                let meta = self.session_metadata(*id, cx);
                if meta.summary.archived {
                    return None;
                }
                let duration_ms = if status.unread_done() {
                    self.sessions.get(id).and_then(|view| {
                        view.read(cx)
                            .transcript
                            .read(cx)
                            .live_turn(&meta.summary.cwd)
                            .2
                    })
                } else {
                    None
                };
                Some(LiveAgent {
                    id: *id,
                    cwd: meta.summary.cwd.clone(),
                    provider: meta.summary.provider.clone(),
                    title: meta.display_title(),
                    label,
                    activity: status.activity.clone(),
                    duration_ms,
                })
            })
            .collect();
        agents.sort_by_key(|agent| match agent.activity {
            Activity::NeedsYou { .. } => (0, 0),
            Activity::Running { since, .. } => (1, since),
            Activity::BackgroundWait { since, .. } => (2, since),
            Activity::Done { at, .. } => (3, at),
            Activity::Idle => (4, 0),
        });
        agents
    }

    pub(super) fn sync_agent_status_tick(&mut self, cx: &mut Context<Self>) {
        if self.live_tick.is_some() || !self.has_running_status() {
            return;
        }
        self.live_tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(STATUS_TICK).await;
                let result = this.update(cx, |this, cx| {
                    this.refresh_status_clocks(cx);
                    if crate::pulse::app_active(cx) {
                        this.invalidate_chrome(cx);
                        cx.notify();
                    }
                    let ticking = this.has_running_status();
                    if !ticking {
                        this.live_tick = None;
                    }
                    ticking
                });
                if !matches!(result, Ok(true)) {
                    break;
                }
            }
        }));
    }

    pub(super) fn live_agents_preview(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let now = monocode_core::time::now_ms();
        let agents = if self.workspace.preferences.working_agents
            && self.workspace.sidebar_activity_visible
        {
            self.live_agents(now, cx)
        } else {
            Vec::new()
        };
        let shown = agents.len() >= LIVE_AGENT_MIN;
        self.live_marks.retain(|id, _| {
            shown
                && agents.iter().any(|agent| {
                    agent.id == *id
                        && matches!(agent.activity, Activity::Running { stale: false, .. })
                })
        });
        if !shown {
            self.live_agents_expanded = false;
            return None;
        }
        let count = agents.len();
        let extra = count.saturating_sub(LIVE_AGENT_CAP);
        let expanded = self.live_agents_expanded;
        let cards: Vec<_> = agents
            .into_iter()
            .take(if expanded { count } else { LIVE_AGENT_CAP })
            .map(|agent| self.live_agent_card(agent, cx))
            .collect();
        Some(
            div()
                .id("live-agents")
                .debug_selector(|| "sidebar-activity-card".into())
                .role(Role::Group)
                .aria_label(format!("{count} working agents"))
                .flex_shrink_0()
                .px_2()
                .when(self.workspace.boxy_layout(), |d| d.pb_2())
                .child(
                    div()
                        .overflow_hidden()
                        .rounded_lg()
                        .bg(content(0.05))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .px(px(14.))
                                .py_1p5()
                                .child(
                                    div()
                                        .size(px(6.))
                                        .flex_shrink_0()
                                        .rounded_full()
                                        .bg(accent_color())
                                        .shadow(vec![
                                            BoxShadow::new(px(0.), px(0.), accent_color())
                                                .blur_radius(px(8.)),
                                        ]),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(12.))
                                        .text_color(content(0.5))
                                        .child("Working"),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(content(0.4))
                                        .child(count.to_string()),
                                ),
                        )
                        .child(
                            div()
                                .id("live-agents-list")
                                .flex()
                                .flex_col()
                                .gap_px()
                                .px_1()
                                .when(extra == 0, |d| d.pb_1())
                                .when(expanded, |d| d.max_h(px(360.)).overflow_y_scroll())
                                .children(cards),
                        )
                        .when(extra > 0, |d| {
                            d.child(
                                div()
                                    .id("live-agents-more")
                                    .group("live-agents-more")
                                    .role(Role::Button)
                                    .aria_expanded(expanded)
                                    .cursor_pointer()
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .gap_1()
                                    .px_2()
                                    .py_1p5()
                                    .text_size(px(11.))
                                    .text_color(content(0.5))
                                    .hover(|s| s.bg(content(0.08)).text_color(content(1.)))
                                    .child(
                                        icon(
                                            if expanded {
                                                "chevron-up"
                                            } else {
                                                "chevron-down"
                                            },
                                            12.,
                                        )
                                        .text_color(content(0.5))
                                        .group_hover("live-agents-more", |s| {
                                            s.text_color(content(1.))
                                        }),
                                    )
                                    .child(if expanded {
                                        "Show less".to_owned()
                                    } else {
                                        format!("{extra} more")
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.live_agents_expanded = !this.live_agents_expanded;
                                        this.notify_sidebar(cx);
                                    })),
                            )
                        }),
                )
                .into_any(),
        )
    }

    fn live_agent_card(&mut self, agent: LiveAgent, cx: &mut Context<Self>) -> AnyElement {
        let id = agent.id;
        let project = self.project_label(&agent.cwd);
        let elapsed = agent.duration_ms.map(live_elapsed);
        let selected = self.selected_sessions.contains(&id)
            || (self.workspace.active == Some(id) && !self.mono_active(cx));
        let status: AnyElement =
            if let Some(glyph) = crate::session_mark::activity_glyph(&agent.activity, 12.) {
                glyph.into_any_element()
            } else {
                let provider = agent.provider.clone();
                let mark = self
                    .live_marks
                    .entry(id)
                    .or_insert_with(|| {
                        cx.new(|cx| {
                            let mut mark = SessionMark::tab(provider.clone());
                            mark.set(&provider, MarkState::Working, cx);
                            mark
                        })
                    })
                    .clone();
                div()
                    .size(px(12.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(mark)
                    .into_any_element()
            };
        let label = [
            Some(agent.title.clone()),
            Some(project.clone()),
            Some(agent.label.clone()),
            elapsed.clone(),
        ]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
        div()
            .id(("live-agent", id as usize))
            .debug_selector(move || format!("live-agent-{id}"))
            .role(Role::Button)
            .aria_label(label)
            .cursor_pointer()
            .w_full()
            .flex()
            .flex_col()
            .rounded_md()
            .px_2()
            .py_1p5()
            .when(selected, |d| d.bg(content(0.1)))
            .when(!selected, |d| d.hover(|s| s.bg(content(0.08))))
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(self.project_mark(&agent.cwd, 8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .line_height(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(content(1.))
                            .child(agent.title),
                    ),
            )
            .child(
                div()
                    .mt_1()
                    .pl_4()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_size(px(11.))
                    .line_height(px(14.))
                    .text_color(crate::session_mark::activity_color(&agent.activity))
                    .child(status)
                    .child(div().min_w_0().truncate().child(agent.label)),
            )
            .child(
                div()
                    .mt_1()
                    .pl_4()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_size(px(11.))
                    .line_height(px(14.))
                    .text_color(content(0.45))
                    .child(provider_icon(&agent.provider, 12.))
                    .child(div().flex_1().min_w_0().truncate().child(project))
                    .children(elapsed.map(|elapsed| div().flex_shrink_0().child(elapsed))),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.selected_sessions.clear();
                this.selection_anchor = Some(id);
                this.open(id, window, cx);
                this.invalidate_chrome(cx);
                cx.notify();
            }))
            .into_any()
    }
}
