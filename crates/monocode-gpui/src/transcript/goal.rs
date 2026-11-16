//! Goal lifecycle rows: one single-line system row between hairlines for
//! state changes, a small accent dot with dimmed text for periodic check-ins.
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
            // The heartbeat of an active goal stays out of the way: a small
            // dot and one dimmed line, left-aligned with the turns it belongs
            // to, with no rules pulling the eye across the transcript.
            return div()
                .w_full()
                .min_w_0()
                .py(px(3.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .size(px(4.))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(accent_color()),
                )
                .child(
                    div()
                        .id(("goal-checkin", message_start))
                        .role(Role::Group)
                        .aria_label(format!("Goal check-in: {}", notice.notice))
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(content(0.4))
                        .child(notice.notice.clone()),
                )
                .into_any_element();
        }
        // One line between hairlines, in the interjection family: a phase
        // label and, after a separator dot, the condition it carried. The
        // notice itself is not repeated below; it said what the label says.
        let (label, color) = match notice.phase {
            GoalPhase::Achieved => ("Goal achieved", success_color()),
            GoalPhase::Error => ("Goal error", danger_color()),
            GoalPhase::Paused => ("Goal paused", warning_color()),
            GoalPhase::Cleared => ("Goal cleared", content(0.55)),
            _ => ("Goal set", content(0.55)),
        };
        let detail = detail_of(notice);
        div()
            .w_full()
            .min_w_0()
            .py_4()
            .flex()
            .items_center()
            .gap_3()
            .child(div().h(px(1.)).flex_1().bg(content(0.12)))
            .child(
                div()
                    .id(("goal-label", message_start))
                    .role(Role::Group)
                    .aria_label(format!("Goal notice: {label}"))
                    .flex()
                    .items_baseline()
                    .gap(px(6.))
                    .min_w_0()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(12.))
                            .text_color(color)
                            .child(label.to_owned()),
                    )
                    .when_some(detail, |d, detail| {
                        d.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .max_w(px(420.))
                                .text_size(px(12.))
                                .text_color(content(0.7))
                                .child(detail),
                        )
                    }),
            )
            .child(div().h(px(1.)).flex_1().bg(content(0.12)))
            .into_any_element()
    }
}

/// The words worth keeping after the phase label: the condition for state
/// changes that carry one, the CLI's reason for a pause or a fatal clear.
fn detail_of(notice: &GoalNotice) -> Option<String> {
    if let Some(condition) = notice.condition.as_deref().filter(|c| !c.is_empty()) {
        return Some(condition.to_owned());
    }
    let prefixes: &[&str] = match notice.phase {
        GoalPhase::Paused => &["Goal paused", "Goal could not be achieved"],
        GoalPhase::Error => &["Goal cleared after an unrecoverable error"],
        GoalPhase::Achieved => &["Goal achieved"],
        _ => return None,
    };
    let rest = prefixes
        .iter()
        .find_map(|prefix| notice.notice.strip_prefix(prefix))
        .unwrap_or(&notice.notice)
        .trim_start_matches([':', '·', ' ', '—'])
        .trim()
        .to_owned();
    (!rest.is_empty()).then_some(rest)
}
