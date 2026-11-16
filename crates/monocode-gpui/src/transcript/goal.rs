//! Goal lifecycle rows: a labelled system row for state changes, a thin
//! hairline divider for periodic check-ins.
use super::*;
use monocode_core::transcript::{GoalNotice, GoalPhase};

impl TranscriptView {
    pub(super) fn goal_row(
        &self,
        message_start: usize,
        notice: &GoalNotice,
        _cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if notice.phase == GoalPhase::Checkin {
            // The heartbeat of an active goal stays out of the way: one thin
            // line of dimmed text between the turns it belongs to.
            return div()
                .w_full()
                .min_w_0()
                .py_1()
                .flex()
                .items_center()
                .gap_2()
                .child(div().h(px(1.)).flex_1().bg(content(0.08)))
                .child(
                    div()
                        .id(("goal-checkin", message_start))
                        .role(Role::Group)
                        .aria_label(format!("Goal check-in: {}", notice.notice))
                        .text_size(px(11.))
                        .text_color(content(0.4))
                        .child(notice.notice.clone()),
                )
                .child(div().h(px(1.)).flex_1().bg(content(0.08)))
                .into_any_element();
        }
        let (label, color) = match notice.phase {
            GoalPhase::Achieved => ("Goal achieved", rgb(0x4ade80).into()),
            GoalPhase::Error => ("Goal error", rgb(0xf87171).into()),
            GoalPhase::Paused => ("Goal paused", content(0.55)),
            GoalPhase::Cleared => ("Goal cleared", content(0.55)),
            _ => ("Goal set", content(0.55)),
        };
        div()
            .w_full()
            .min_w_0()
            .py_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().h(px(1.)).flex_1().bg(content(0.12)))
                    .child(
                        div()
                            .id(("goal-label", message_start))
                            .role(Role::Group)
                            .aria_label(format!("Goal notice: {label}"))
                            .text_size(px(12.))
                            .text_color(color)
                            .child(label.to_owned()),
                    )
                    .child(div().h(px(1.)).flex_1().bg(content(0.12))),
            )
            .when(!notice.notice.is_empty(), |d| {
                d.child(
                    div()
                        .w_full()
                        .text_size(px(12.5))
                        .line_height(px(20.))
                        .text_color(content(0.7))
                        .child(notice.notice.clone()),
                )
            })
            .into_any_element()
    }
}
