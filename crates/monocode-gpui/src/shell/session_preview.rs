//! Hover cards for unopened sessions do not construct conversation entities.
use crate::{pickers::cached_model_name, session_mark::activity_color, theme::*};
use gpui::{AnyElement, FontWeight, div, prelude::*, px};
use monocode_core::{
    agent_status::{Activity, AgentStatus},
    provider::Kind,
    store::{Session, SessionPreviewData},
};
use std::path::Path;

pub(super) fn render(
    summary: &Session,
    data: Option<&SessionPreviewData>,
    status: &AgentStatus,
    tree: bool,
) -> AnyElement {
    let model = data
        .and_then(|data| data.model.as_deref())
        .and_then(|model| monocode_core::store::native_model(model, &summary.provider));
    let name = cached_model_name(Kind::from_id(&summary.provider), model.as_deref());
    let worktree = data.and_then(|data| data.worktree_cwd.as_deref());
    let cwd = worktree.unwrap_or(&summary.cwd);
    let now = monocode_core::time::now_ms();
    let label = preview_label(summary, status, tree, now);
    let directory = std::env::var_os("HOME")
        .and_then(|home| {
            Path::new(cwd).strip_prefix(home).ok().map(|path| {
                if path.as_os_str().is_empty() {
                    "~".to_owned()
                } else {
                    format!("~/{}", path.display())
                }
            })
        })
        .unwrap_or_else(|| cwd.to_owned());
    div()
        .px_3()
        .py_2p5()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .text_size(px(11.))
                        .text_color(content(0.55))
                        .child(provider_icon(&summary.provider, 14.))
                        .child(div().min_w_0().truncate().child(name)),
                )
                .child(
                    div()
                        .max_w(gpui::relative(0.65))
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(activity_color(&status.activity))
                        .child(label),
                ),
        )
        .child(
            div()
                .mt_1p5()
                .text_size(px(13.))
                .line_height(px(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(content(1.))
                .child(
                    monocode_core::session_title::display_title(&summary.title, &summary.provider)
                        .replace('\n', " "),
                ),
        )
        .child(
            div()
                .mt_2()
                .pt_2()
                .border_t_1()
                .border_color(content(0.08))
                .text_size(px(11.))
                .line_height(px(16.))
                .flex()
                .gap_2()
                .child(
                    div()
                        .w(px(72.))
                        .flex_shrink_0()
                        .text_color(content(0.45))
                        .child(if worktree.is_some() {
                            "Worktree"
                        } else {
                            "Directory"
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(content(0.8))
                        .child(directory),
                ),
        )
        .into_any_element()
}

fn preview_label(summary: &Session, status: &AgentStatus, tree: bool, now: u64) -> String {
    if tree && let Some(label) = crate::session_mark::finished_preview_label(&status.activity, now)
    {
        return label;
    }
    status.label(now).unwrap_or_else(|| {
        let at = match status.activity {
            Activity::Done { at, .. } => at,
            _ => summary.updated_at,
        };
        monocode_core::time::relative(at, now)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use monocode_core::{agent_status::Outcome, store::Store};

    #[test]
    fn unopened_tree_preview_uses_the_receipt_even_when_the_session_was_updated_later() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let mut summary = store.create("claude", "/work/app").unwrap();
        let now = 5_000_000;
        summary.updated_at = now - 2_000;
        for unread in [true, false] {
            let status = AgentStatus {
                activity: Activity::Done {
                    at: now - 79 * 60_000,
                    outcome: Outcome::Completed,
                    unread,
                },
                ..Default::default()
            };
            assert_eq!(
                preview_label(&summary, &status, true, now),
                "Finished 1h 19m ago"
            );
            if !unread {
                assert_eq!(preview_label(&summary, &status, false, now), "1h 19m");
            }
        }
    }
}
