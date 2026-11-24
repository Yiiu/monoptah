//! SettingsView.tsx's building blocks: PageHeader, Group, Row, Toggle,
//! Segmented, Slider, Select and SecondaryButton, at their Tailwind metrics.
use super::{SettingsView, SliderKind};
use crate::{
    popover::{Align, Anchor, AnchorSlot, Side, popover},
    theme::*,
};
use gpui::{
    AnyElement, Context, Div, ElementId, Entity, FontWeight, Hsla, MouseButton, MouseDownEvent,
    Role, SharedString, Stateful, div, prelude::*, px, relative, rgb,
};

pub(super) const RED: u32 = 0xf87171;
/// `text-[12px] leading-relaxed` and `text-[13px]` line boxes.
const RELAXED_12: f32 = 19.5;

pub(super) fn red(alpha: f32) -> Hsla {
    let mut color: Hsla = rgb(RED).into();
    color.a = alpha;
    color
}
pub(super) fn accent(alpha: f32) -> Hsla {
    let mut color: Hsla = accent_color();
    color.a = alpha;
    color
}

/// PageHeader: the page's title and what it is for.
pub(super) fn page_header(title: &'static str, description: &'static str) -> Div {
    div()
        .debug_selector(|| "settings-page-header".into())
        .pb_4()
        .child(
            div()
                .text_size(px(20.))
                .line_height(px(25.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(content(1.))
                .child(title),
        )
        .child(
            div()
                .mt(px(6.))
                .max_w(px(576.))
                .text_size(px(13.))
                .line_height(px(21.125))
                .text_color(content(0.45))
                .child(description),
        )
}

/// A muted line inside a group card: an empty state or a hint.
pub(super) fn note(text: impl Into<SharedString>) -> AnyElement {
    div()
        .px_4()
        .py(px(14.))
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(content(0.45))
        .child(text.into())
        .into_any_element()
}

/// SecondaryButton.tsx.
pub(crate) fn secondary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    danger: bool,
    disabled: bool,
) -> Stateful<Div> {
    button_base(id, label).map(|d| {
        if disabled {
            d.opacity(0.4).text_color(if danger {
                rgb(RED).into()
            } else {
                content(0.7)
            })
        } else if danger {
            d.cursor_pointer()
                .text_color(rgb(RED))
                .hover(|s| s.border_color(red(0.4)).bg(red(0.1)))
        } else {
            d.cursor_pointer()
                .text_color(content(0.7))
                .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
        }
    })
}

/// A 14px checkbox, like the ones in the delete dialogs. The caller adds
/// `on_click` when it is enabled.
pub(crate) fn checkbox(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    checked: bool,
    disabled: bool,
) -> Stateful<Div> {
    let id = id.into();
    let selector = id.to_string();
    div()
        .id(id)
        .debug_selector(move || selector.clone())
        .role(Role::CheckBox)
        .aria_label(label.into())
        .aria_toggled(if checked {
            gpui::Toggled::True
        } else {
            gpui::Toggled::False
        })
        .mt(px(2.))
        .size(px(14.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(if checked {
            accent_color()
        } else {
            content(0.3)
        })
        .when(checked, |d| {
            d.bg(accent_color())
                .child(icon("check", 10.).text_color(gpui::white()))
        })
        .when(disabled, |d| d.opacity(0.4))
        .when(!disabled, |d| d.cursor_pointer())
}

fn button_base(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    let label = label.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(6.))
        .rounded_md()
        .border_1()
        .border_color(content(0.1))
        .px(px(10.))
        .py_1()
        .text_size(px(12.))
        .line_height(px(16.))
        .whitespace_nowrap()
}

/// Borderless, square icon actions used by SkillsPage and MonoRow.
pub(super) fn icon_action(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    path: &'static str,
    side: f32,
    icon_side: f32,
    alpha: f32,
    disabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.into())
        .group("settings-icon-action")
        .size(px(side))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(if side == 20. { 4. } else { 6. }))
        .text_color(content(alpha))
        .when(disabled, |d| d.opacity(0.4))
        .when(!disabled, |d| {
            d.cursor_pointer()
                .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
        })
        .child(
            icon(path, icon_side)
                .text_color(content(alpha))
                .when(!disabled, |i| {
                    i.group_hover("settings-icon-action", |s| s.text_color(content(1.)))
                }),
        )
}

pub(crate) fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    disabled: bool,
) -> Stateful<Div> {
    let mut ink = shell_fill();
    ink.a = 1.;
    button_base(id, label)
        .border_0()
        .bg(content(1.))
        .text_color(ink)
        .font_weight(gpui::FontWeight::MEDIUM)
        .map(|d| {
            if disabled {
                d.opacity(0.4)
            } else {
                d.cursor_pointer()
                    .hover(move |s| s.bg(content(0.85)).text_color(ink))
            }
        })
}

impl SettingsView {
    pub(super) fn flashing(&self, id: Option<&str>) -> bool {
        id.is_some() && self.revealed.as_deref() == id
    }

    /// Group: a titled card. `first` drops the `pt-8` the CSS `first:` does.
    pub(super) fn group(
        &self,
        id: Option<&'static str>,
        first: bool,
        title: impl IntoElement,
        description: Option<SharedString>,
        action: Option<AnyElement>,
        children: Vec<AnyElement>,
    ) -> AnyElement {
        let flash = self.flashing(id);
        div()
            .relative()
            .when(!first, |d| d.pt_8())
            .when_some(id, |d, id| d.child(self.reveal_probe(id)))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_4()
                    .pb(px(10.))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .line_height(px(19.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(content(1.))
                                    .child(title),
                            )
                            .when_some(description, |d, text| {
                                d.child(
                                    div()
                                        .mt_1()
                                        .text_size(px(12.))
                                        .line_height(px(RELAXED_12))
                                        .text_color(content(0.45))
                                        .child(text),
                                )
                            }),
                    )
                    .when_some(action, |d, action| {
                        d.child(div().flex_shrink_0().pb(px(2.)).child(action))
                    }),
            )
            .child(
                div()
                    .overflow_hidden()
                    .rounded_xl()
                    .border_1()
                    .border_color(if flash { accent(0.6) } else { content(0.1) })
                    .bg(content(0.03))
                    .flex()
                    .flex_col()
                    .children(children),
            )
            .into_any_element()
    }

    /// Row: a label and description beside its controls. Rows draw their
    /// own top rule, which reads the same as Tailwind's `border-b` with
    /// `last:border-b-0`.
    pub(super) fn row(
        &self,
        id: Option<&'static str>,
        first: bool,
        label: impl IntoElement,
        description: Option<SharedString>,
        controls: Vec<AnyElement>,
    ) -> AnyElement {
        let flash = self.flashing(id);
        let narrow =
            self.scroll.bounds().size.width > px(0.) && self.scroll.bounds().size.width < px(560.);
        let toggle_only = id.is_some_and(|id| {
            matches!(
                id,
                "sounds"
                    | "notifications"
                    | "tab-animations"
                    | "notes"
                    | "quick-composer"
                    | "working-agents"
                    | "action-opens-terminal"
                    | "close-to-tray"
                    | "main-pane-glass"
                    | "show-excluded-files"
                    | "anchor-prompts"
                    | "paced-reveal"
                    | "collapse-tool-groups"
                    | "fold-finished-work"
                    | "composer-autocorrect"
                    | "format-on-save"
                    | "inline-blame"
                    | "composer-mascot"
                    | "empty-session-games"
                    | "show-remaining-usage"
                    | "mask-emails"
                    | "claude-hooks"
                    | "auto-resume-usage-limit"
                    | "claude-entrypoint"
                    | "monos-enabled"
                    | "rail-monos-pinned"
                    | "mono-menu-bar-icon"
                    | "show-archived"
            )
        });
        let stacked = narrow && !toggle_only;
        div()
            .debug_selector(|| format!("settings-row-{}", id.unwrap_or("anonymous")))
            .relative()
            .flex()
            .items_start()
            .map(|d| {
                if stacked {
                    d.flex_col().gap_3()
                } else {
                    d.gap_6()
                }
            })
            .px_4()
            .py(px(14.))
            .when(!first, |d| d.border_t_1().border_color(content(0.05)))
            .when(flash, |d| d.bg(accent(0.1)))
            .when_some(id, |d, id| d.child(self.reveal_probe(id)))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(19.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(content(1.))
                            .child(label),
                    )
                    .when_some(description, |d, text| {
                        d.child(
                            div()
                                .mt_1()
                                .text_size(px(12.))
                                .line_height(px(RELAXED_12))
                                .text_color(content(0.45))
                                .child(text),
                        )
                    }),
            )
            .child(
                div()
                    .min_w_0()
                    .map(|d| {
                        if stacked {
                            d.max_w_full()
                        } else {
                            d.max_w(relative(0.6))
                        }
                    })
                    .flex_shrink_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .map(|d| {
                        if stacked {
                            d.justify_start()
                        } else {
                            d.justify_end()
                        }
                    })
                    .gap_2()
                    .children(controls),
            )
            .into_any_element()
    }

    /// Where search lands: each target reports its place in the page.
    pub(in crate::settings) fn reveal_probe(&self, id: &'static str) -> AnyElement {
        let slot = self.targets.borrow_mut().entry(id).or_default().clone();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(slot.layout_probe())
            .into_any_element()
    }

    /// Toggle: a 36×20 switch with a 16px knob.
    pub(super) fn toggle(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        on: bool,
        disabled: bool,
        cx: &mut Context<Self>,
        change: impl Fn(&mut Self, bool, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        div()
            .id(SharedString::from(format!("toggle-{id}")))
            .debug_selector(move || format!("settings-toggle-{id}"))
            .role(Role::Switch)
            .aria_label(label.into())
            .aria_toggled(if on {
                gpui::Toggled::True
            } else {
                gpui::Toggled::False
            })
            .relative()
            .h(px(20.))
            .w(px(36.))
            .flex_shrink_0()
            .rounded_full()
            .bg(if on { accent_color() } else { content(0.2) })
            .when(disabled, |d| d.opacity(0.4))
            .child(
                div()
                    .absolute()
                    .top(px(2.))
                    .left(px(if on { 18. } else { 2. }))
                    .size(px(16.))
                    .rounded_full()
                    .bg(rgb(0xffffff)),
            )
            .when(!disabled, |d| {
                d.cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| change(this, !on, cx)))
            })
            .into_any_element()
    }

    /// Segmented: equal-width options in a hairline frame.
    pub(super) fn segmented(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        value: &str,
        options: &[(&'static str, &str)],
        cx: &mut Context<Self>,
        change: impl Fn(&mut Self, &'static str, &mut Context<Self>) + Clone + 'static,
    ) -> AnyElement {
        let options: Vec<(&'static str, &str, Option<&'static str>)> = options
            .iter()
            .map(|&(option, text)| (option, text, None))
            .collect();
        self.segmented_icons(id, label, value, &options, cx, change)
    }

    /// Segmented whose options may lead with a 12px icon.
    pub(super) fn segmented_icons(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        value: &str,
        options: &[(&'static str, &str, Option<&'static str>)],
        cx: &mut Context<Self>,
        change: impl Fn(&mut Self, &'static str, &mut Context<Self>) + Clone + 'static,
    ) -> AnyElement {
        let id: SharedString = id.into();
        div()
            .id(SharedString::from(format!("segmented-{id}")))
            .role(Role::RadioGroup)
            .aria_label(label.into())
            .grid()
            .grid_cols(options.len() as u16)
            .flex_shrink_0()
            .gap(px(2.))
            .rounded_md()
            .border_1()
            .border_color(content(0.1))
            .p(px(2.))
            .text_size(px(12.))
            .line_height(px(16.))
            .children(options.iter().map(|&(option, text, glyph)| {
                let selected = value == option;
                let change = change.clone();
                let id = id.clone();
                div()
                    .id(SharedString::from(format!("segmented-{id}-{option}")))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("settings-{id}-{option}")
                    })
                    .role(Role::RadioButton)
                    .aria_label(text.to_owned())
                    .aria_selected(selected)
                    .min_w_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .whitespace_nowrap()
                    .rounded(px(5.))
                    .px(px(10.))
                    .py_1()
                    .cursor_pointer()
                    .map(|d| {
                        if selected {
                            d.bg(content(0.1)).text_color(content(1.))
                        } else {
                            d.text_color(content(0.5))
                                .hover(|s| s.text_color(content(1.)))
                        }
                    })
                    .when_some(glyph, |d, glyph| {
                        d.child(icon(glyph, 12.).text_color(content(if selected {
                            1.
                        } else {
                            0.5
                        })))
                    })
                    .child(text.to_owned())
                    .on_click(cx.listener(move |this, _, _, cx| change(this, option, cx)))
            }))
            .into_any_element()
    }

    /// Slider: a 4px track and 12px thumb, then its value, in a 224px row.
    pub(super) fn slider(
        &self,
        kind: SliderKind,
        display: String,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (min, max) = kind.range();
        let value = kind.get(&self.prefs);
        let fraction = if max > min {
            (value.saturating_sub(min)) as f32 / (max - min) as f32
        } else {
            0.
        };
        let slot = self
            .slider_tracks
            .borrow_mut()
            .entry(kind)
            .or_default()
            .clone();
        div()
            .w(px(224.))
            .max_w_full()
            .flex()
            .items_center()
            .gap_3()
            .when(disabled, |d| d.opacity(0.4))
            .child(
                div()
                    .id(SharedString::from(format!("slider-{}", kind.id())))
                    .debug_selector(move || format!("settings-slider-{}", kind.id()))
                    .role(Role::Slider)
                    .aria_label(kind.label())
                    .min_w_0()
                    .flex_1()
                    .h(px(12.))
                    .relative()
                    .flex()
                    .items_center()
                    .child(slot.layout_probe())
                    .child(div().w_full().h(px(4.)).rounded_full().bg(content(0.15)))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left(relative(fraction))
                            .ml(px(-6. + 12. * (0.5 - fraction)))
                            .size(px(12.))
                            .rounded_full()
                            .bg(content(1.)),
                    )
                    .when(!disabled, |d| {
                        d.cursor_pointer().on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                this.slider_drag = Some(kind);
                                this.slide(kind, event.position.x, cx);
                                cx.stop_propagation();
                            }),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(40.))
                    .flex_shrink_0()
                    .text_right()
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(content(1.))
                    .child(display),
            )
            .into_any_element()
    }
    pub(super) fn slide(&mut self, kind: SliderKind, x: gpui::Pixels, cx: &mut Context<Self>) {
        let Some(bounds) = self.slider_tracks.borrow().get(&kind).and_then(|s| s.get()) else {
            return;
        };
        let (min, max) = kind.range();
        // A range input's thumb travels the track less its own width.
        let travel = (bounds.size.width.as_f32() - 12.).max(1.);
        let fraction = ((x - bounds.left()).as_f32() - 6.).clamp(0., travel) / travel;
        let value = min + ((max - min) as f32 * fraction).round() as u32;
        if kind.get(&self.prefs) != value {
            self.update(cx, |p| kind.set(p, value));
        }
    }

    /// Select: a bordered trigger opening a listbox popover below it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn select(
        &self,
        id: &'static str,
        label: &'static str,
        value: &str,
        options: Vec<(String, SharedString, Option<AnyElement>)>,
        width: Option<f32>,
        cx: &mut Context<Self>,
        change: impl Fn(&mut Self, String, &mut Context<Self>) + Clone + 'static,
    ) -> AnyElement {
        let open = self.select_open == Some(id);
        let slot: AnchorSlot = self
            .select_anchors
            .borrow_mut()
            .entry(id)
            .or_default()
            .clone();
        let selected = options.iter().position(|(v, _, _)| v == value);
        let mut options = options;
        let (trigger_label, trigger_icon) = match selected {
            Some(ix) => {
                let (_, text, icon) = &mut options[ix];
                (text.clone(), icon.take())
            }
            None => (SharedString::from(value.to_owned()), None),
        };
        // Long lists get a filter field; the list shows only matching rows.
        let searchable = options.len() > SEARCHABLE_SELECT;
        let query = if open && searchable {
            self.select_query.trim().to_lowercase()
        } else {
            String::new()
        };
        if !query.is_empty() {
            options.retain(|(_, text, _)| text.to_lowercase().contains(&query));
        }
        let values: Vec<String> = options.iter().map(|(v, _, _)| v.clone()).collect();
        let weak = cx.weak_entity();
        div()
            .relative()
            .map(|d| match width {
                Some(w) => d.w(px(w)),
                None => d.max_w(px(208.)),
            })
            .child(
                div()
                    .id(SharedString::from(format!("select-{id}")))
                    .debug_selector(move || format!("settings-select-{id}"))
                    .role(Role::ComboBox)
                    .aria_label(format!("{label}: {trigger_label}"))
                    .aria_expanded(open)
                    .relative()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .rounded_md()
                    .border_1()
                    .border_color(content(0.1))
                    .bg(content(0.05))
                    .px_2()
                    .py_1()
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(content(1.))
                    .cursor_pointer()
                    .hover(|s| s.border_color(content(0.2)))
                    .child(slot.probe())
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .when_some(trigger_icon, |d, icon| {
                                d.child(
                                    div()
                                        .size_4()
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(icon),
                                )
                            })
                            .child(div().min_w_0().truncate().child(trigger_label)),
                    )
                    .child(
                        icon("chevron-down-hi", 14.)
                            .text_color(content(0.5))
                            .when(open, |s| {
                                s.with_transformation(gpui::Transformation::rotate(
                                    gpui::percentage(0.5),
                                ))
                            }),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.select_open == Some(id) {
                            this.select_open = None;
                        } else {
                            this.select_open = Some(id);
                            this.select_active = selected.unwrap_or(0);
                            this.select_scroll.scroll_to_item(this.select_active);
                            this.select_query.clear();
                            if searchable {
                                this.select_filter
                                    .update(cx, |i, cx| i.set_text(String::new(), cx));
                                window.focus(
                                    &gpui::Focusable::focus_handle(&this.select_filter, cx),
                                    cx,
                                );
                            } else {
                                window.focus(&this.select_focus, cx);
                            }
                        }
                        cx.notify();
                    })),
            )
            .when(open, |d| {
                let active = self.select_active;
                let rows = options
                    .into_iter()
                    .enumerate()
                    .map(|(ix, (option, text, icon_el))| {
                        let is_selected = option == value;
                        let highlighted = ix == active;
                        let change = change.clone();
                        div()
                            .id(SharedString::from(format!("select-{id}-{ix}")))
                            .debug_selector(move || format!("settings-option-{id}-{ix}"))
                            .role(Role::ListBoxOption)
                            .aria_label(text.clone())
                            .aria_selected(is_selected)
                            .w_full()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded_lg()
                            .px_2()
                            .py(px(6.))
                            .text_size(px(12.))
                            .line_height(px(16.))
                            .text_color(content(1.))
                            .cursor_pointer()
                            .map(|d| {
                                if highlighted || is_selected {
                                    d.bg(content(0.1))
                                } else {
                                    d.hover(|s| s.bg(content(0.05)))
                                }
                            })
                            .when_some(icon_el, |d, icon_el| {
                                d.child(
                                    div()
                                        .size_4()
                                        .flex_shrink_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(icon_el),
                                )
                            })
                            .child(div().min_w_0().flex_1().truncate().child(text))
                            .when(is_selected, |d| {
                                d.child(icon("check-hi", 14.).text_color(content(1.)))
                            })
                            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                                if this.select_active != ix {
                                    this.select_active = ix;
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_open = None;
                                change(this, option.clone(), cx);
                                window.focus(&this.focus, cx);
                                cx.notify();
                            }))
                    })
                    .collect::<Vec<_>>();
                let count = rows.len();
                let filter = searchable.then(|| {
                    div()
                        .flex_shrink_0()
                        .h(px(28.))
                        .mb_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .rounded_md()
                        .border_1()
                        .border_color(content(0.1))
                        .px_2()
                        .child(icon("search-01", 14.).text_color(content(0.45)))
                        .child(div().min_w_0().flex_1().child(self.select_filter.clone()))
                });
                let keys_change = change.clone();
                d.child(
                    popover(
                        SharedString::from(format!("select-popover-{id}")),
                        Anchor::Slot(slot),
                    )
                    .side(Side::Bottom)
                    .align(Align::End)
                    .width(280.)
                    .max_height(320.)
                    .padding(4.)
                    .on_dismiss(move |window, cx| {
                        let _ = weak.update(cx, |this, cx| {
                            this.select_open = None;
                            window.focus(&this.focus, cx);
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .children(filter)
                            .child(
                                div()
                                    .id(SharedString::from(format!("select-list-{id}")))
                                    .role(Role::ListBox)
                                    .aria_label(label)
                                    .track_focus(&self.select_focus)
                                    .track_scroll(&self.select_scroll)
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .flex()
                                    .flex_col()
                                    .children(rows)
                                    .when(count == 0, |d| {
                                        d.child(
                                            div()
                                                .px_2()
                                                .py(px(6.))
                                                .text_size(px(12.))
                                                .line_height(px(16.))
                                                .text_color(content(0.5))
                                                .child("No matches"),
                                        )
                                    }),
                            )
                            .on_key_down(cx.listener(
                                move |this, event: &gpui::KeyDownEvent, window, cx| {
                                    let key = event.keystroke.key.as_str();
                                    let last = count.saturating_sub(1);
                                    match key {
                                        "down" => {
                                            this.select_active = (this.select_active + 1).min(last)
                                        }
                                        "up" => {
                                            this.select_active =
                                                this.select_active.saturating_sub(1)
                                        }
                                        "home" => this.select_active = 0,
                                        "end" => this.select_active = last,
                                        "enter" | "tab" => {
                                            if let Some(v) = values.get(this.select_active) {
                                                keys_change(this, v.clone(), cx);
                                            }
                                            this.select_open = None;
                                            window.focus(&this.focus, cx);
                                        }
                                        "escape" => {
                                            this.select_open = None;
                                            window.focus(&this.focus, cx);
                                        }
                                        _ => return,
                                    }
                                    this.select_scroll.scroll_to_item(this.select_active);
                                    cx.stop_propagation();
                                    cx.notify();
                                },
                            )),
                    ),
                )
            })
            .into_any_element()
    }
}

/// Selects with more options than this get a filter field.
const SEARCHABLE_SELECT: usize = 8;

impl SettingsView {
    pub(super) fn project_picker(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let options = self
            .ctx
            .projects
            .iter()
            .map(|(path, label)| {
                (
                    path.clone(),
                    SharedString::from(label.clone()),
                    Some(icon("folder", 14.).into_any_element()),
                )
            })
            .collect();
        let value = self.selected_project.clone();
        self.select(
            "settings-project",
            "Project",
            &value,
            options,
            None,
            cx,
            |this, value, cx| {
                this.selected_project = value;
                this.selected_trees.clear();
                this.refresh(cx);
            },
        )
    }
    pub(super) fn refresh_button(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        secondary_button(id, label, false, self.discovery_loading)
            .child(icon("refresh-cw", 14.))
            .child(label)
            .when(!self.discovery_loading, |d| {
                d.on_click(cx.listener(|this, _, _, cx| this.refresh(cx)))
            })
            .into_any_element()
    }
}

// --- Building blocks first used by the Agent CLIs settings. They are stateless, take their
// colours from the theme and keep the metrics of the widgets above.

/// `color` at another opacity: the fill behind a badge, an icon tile or a banner.
pub(crate) fn tinted(color: Hsla, alpha: f32) -> Hsla {
    let mut color = color;
    color.a = alpha;
    color
}

/// A small tag: an optional 12px icon and 11px medium text on a 13% tint of `color`.
/// Used for Default, Off and permission badges.
pub(crate) fn badge(
    text: impl Into<SharedString>,
    glyph: Option<&'static str>,
    color: Hsla,
) -> Div {
    let ink = tinted(color, 1.);
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap_1()
        .h(px(18.))
        .rounded(px(4.))
        .bg(tinted(color, 0.13))
        .px(px(6.))
        .text_size(px(11.))
        .line_height(px(16.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(ink)
        .whitespace_nowrap()
        .when_some(glyph, |d, glyph| d.child(icon(glyph, 12.).text_color(ink)))
        .child(text.into())
}

/// A monospace chip, such as one argument of a split argument line.
pub(crate) fn chip(text: impl Into<SharedString>) -> Div {
    div()
        .flex_shrink_0()
        .rounded(px(4.))
        .bg(content(0.08))
        .px(px(6.))
        .font_family(crate::fonts::mono())
        .text_size(px(11.))
        .line_height(px(16.))
        .text_color(content(0.72))
        .child(text.into())
}

/// A square tile holding an icon on a 13% tint of its colour.
pub(crate) fn icon_tile(glyph: &'static str, color: Hsla, side: f32) -> Div {
    div()
        .size(px(side))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .bg(tinted(color, 0.13))
        .child(icon(glyph, (side * 0.6).round()).text_color(tinted(color, 1.)))
}

/// One field of a details panel: a 136px label column beside its controls, which stack.
/// Rows draw their own top rule, as `row` does.
pub(super) fn field(
    first: bool,
    label: impl Into<SharedString>,
    hint: Option<SharedString>,
    body: Vec<AnyElement>,
) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap_4()
        .py_3()
        .when(!first, |d| d.border_t_1().border_color(content(0.05)))
        .child(
            div()
                .w(px(136.))
                .flex_shrink_0()
                .pt(px(4.))
                .child(
                    div()
                        .text_size(px(13.))
                        .line_height(px(19.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(content(1.))
                        .child(label.into()),
                )
                .when_some(hint, |d, hint| {
                    d.child(
                        div()
                            .mt(px(2.))
                            .text_size(px(12.))
                            .line_height(px(16.))
                            .text_color(content(0.45))
                            .child(hint),
                    )
                }),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(6.))
                .children(body),
        )
        .into_any_element()
}

/// A muted help line under a control.
pub(super) fn help(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(content(0.45))
        .child(text.into())
}

/// A validation error under a field.
pub(super) fn field_error(
    id: impl Into<ElementId>,
    selector: String,
    text: impl Into<SharedString>,
) -> AnyElement {
    let text = text.into();
    div()
        .id(id)
        .debug_selector(move || selector.clone())
        .role(Role::Alert)
        .aria_label(text.clone())
        .flex()
        .items_start()
        .gap_1()
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(danger_color())
        .child(icon("circle-x", 12.).mt(px(2.)).text_color(danger_color()))
        .child(div().min_w_0().flex_1().child(text))
        .into_any_element()
}

/// "Saved", shown briefly after a field commits.
pub(super) fn saved_flash() -> AnyElement {
    div()
        .debug_selector(|| "settings-saved".into())
        .flex()
        .items_center()
        .gap_1()
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(success_color())
        .child(icon("check", 12.).text_color(success_color()))
        .child("Saved")
        .into_any_element()
}

/// A status dot followed by short facts; a monospace fact is a path or a version.
pub(super) fn status_line(dot: Hsla, facts: Vec<(SharedString, bool)>) -> AnyElement {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .line_height(px(16.))
        .text_color(content(0.5))
        .child(div().size(px(6.)).flex_shrink_0().rounded_full().bg(dot))
        .children(facts.into_iter().map(|(text, mono)| {
            div()
                .min_w_0()
                .when(mono, |d| {
                    d.font_family(crate::fonts::mono()).text_size(px(11.))
                })
                .child(text)
        }))
        .into_any_element()
}

/// The frame around a one-line text field: 28px, the select trigger's border and fill.
pub(super) fn text_box(input: Entity<crate::input::Input>, mono: bool, invalid: bool) -> Div {
    div()
        .min_w_0()
        .flex_1()
        .h(px(28.))
        .flex()
        .items_center()
        .rounded_md()
        .border_1()
        .border_color(if invalid {
            danger_color()
        } else {
            content(0.1)
        })
        .bg(content(0.05))
        .px_2()
        .when(mono, |d| d.font_family(crate::fonts::mono()))
        .child(div().min_w_0().flex_1().child(input))
}

/// The frame around a multi-line text field.
pub(super) fn text_area(input: Entity<crate::input::Input>, mono: bool, invalid: bool) -> Div {
    div()
        .w_full()
        .rounded_md()
        .border_1()
        .border_color(if invalid {
            danger_color()
        } else {
            content(0.1)
        })
        .bg(content(0.05))
        .px_2()
        .py(px(6.))
        .when(mono, |d| d.font_family(crate::fonts::mono()))
        .child(input)
}

/// A tinted notice inside a card: an icon, a message and an optional action. `ink` is the
/// message colour (the text colour for an error to read, the tint itself for a warning).
pub(super) fn inline_banner(
    id: impl Into<ElementId>,
    glyph: &'static str,
    color: Hsla,
    ink: Hsla,
    text: impl Into<SharedString>,
    action: Option<AnyElement>,
) -> Stateful<Div> {
    let text = text.into();
    div()
        .id(id)
        .role(Role::Alert)
        .aria_label(text.clone())
        .flex()
        .items_start()
        .gap_2()
        .rounded_md()
        .bg(tinted(color, 0.1))
        .px_3()
        .py_2()
        .text_size(px(12.))
        .line_height(px(16.))
        .child(icon(glyph, 14.).mt(px(1.)).text_color(tinted(color, 1.)))
        .child(div().min_w_0().flex_1().text_color(ink).child(text))
        .children(action)
}

/// A placeholder row while a list loads: an icon tile and a text bar.
pub(super) fn skeleton_row() -> AnyElement {
    div()
        .h(px(56.))
        .flex()
        .items_center()
        .gap_3()
        .border_t_1()
        .border_color(content(0.05))
        .px_4()
        .child(div().size(px(28.)).rounded(px(7.)).bg(content(0.08)))
        .child(
            div()
                .w(px(180.))
                .h(px(10.))
                .rounded(px(4.))
                .bg(content(0.08)),
        )
        .into_any_element()
}

/// A monospace block under a small header with an action on the right, such as a launch
/// preview with Copy.
pub(super) fn code_block(
    glyph: &'static str,
    title: impl Into<SharedString>,
    action: AnyElement,
    body: AnyElement,
) -> Div {
    div()
        .rounded_md()
        .border_1()
        .border_color(content(0.1))
        .bg(content(0.03))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .pl_3()
                .pr(px(6.))
                .pt(px(6.))
                .text_size(px(12.))
                .line_height(px(16.))
                .text_color(content(0.5))
                .child(icon(glyph, 13.))
                .child(div().min_w_0().flex_1().child(title.into()))
                .child(action),
        )
        .child(
            div()
                .px_3()
                .pt_1()
                .pb(px(10.))
                .font_family(crate::fonts::mono())
                .text_size(px(12.))
                .line_height(px(19.))
                .text_color(content(0.8))
                .child(body),
        )
}

/// The frame of a read-only variables table; its rows are `env_row`s.
pub(super) fn env_table(rows: Vec<AnyElement>) -> Div {
    div()
        .rounded_md()
        .border_1()
        .border_color(content(0.1))
        .bg(content(0.03))
        .overflow_hidden()
        .flex()
        .flex_col()
        .children(rows)
}

/// One variable: its name, its (possibly masked) value, an optional note such as who
/// overrides it, and a trailing control such as the reveal button.
pub(super) fn env_row(
    first: bool,
    key: impl Into<SharedString>,
    value: impl Into<SharedString>,
    note: Option<SharedString>,
    trailing: Option<AnyElement>,
) -> AnyElement {
    div()
        .min_h(px(30.))
        .flex()
        .items_center()
        .gap_2()
        .pl(px(10.))
        .pr_1()
        .when(!first, |d| d.border_t_1().border_color(content(0.05)))
        .text_size(px(12.))
        .line_height(px(16.))
        .child(
            div()
                .w(px(200.))
                .flex_shrink_0()
                .truncate()
                .font_family(crate::fonts::mono())
                .text_color(content(1.))
                .child(key.into()),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .font_family(crate::fonts::mono())
                .text_color(content(0.55))
                .child(value.into()),
        )
        .when_some(note, |d, note| {
            d.child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .text_size(px(11.))
                    .text_color(warning_color())
                    .child(icon("lock", 11.).text_color(warning_color()))
                    .child(note),
            )
        })
        .child(
            div()
                .size(px(24.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .children(trailing),
        )
        .into_any_element()
}

/// A notice at the bottom right of Settings: an icon, a message and an optional action
/// such as Undo. One shows at a time.
pub(super) fn settings_toast(
    glyph: &'static str,
    text: impl Into<SharedString>,
    action: Option<AnyElement>,
) -> AnyElement {
    let text = text.into();
    crate::notifications::surface::toast_frame()
        .id("settings-toast")
        .debug_selector(|| "settings-toast".into())
        .role(Role::Status)
        .aria_label(text.clone())
        .w(px(380.))
        .child(
            div()
                .relative()
                .flex()
                .items_start()
                .gap(px(10.))
                .px_3()
                .py(px(10.))
                .text_size(px(12.))
                .line_height(px(16.))
                .child(icon(glyph, 16.).text_color(accent_color()))
                .child(div().min_w_0().flex_1().text_color(content(1.)).child(text))
                .children(action),
        )
        .into_any_element()
}

/// One card of `radio_cards`.
pub(super) struct RadioCard {
    pub value: &'static str,
    pub title: SharedString,
    pub description: SharedString,
    /// The compact card's leading icon.
    pub glyph: Option<&'static str>,
    /// The regular card's 56px preview.
    pub preview: Option<AnyElement>,
}

impl SettingsView {
    /// Radio cards: one choice among a few, each with a title and a line of description.
    /// Regular cards sit in equal columns under a preview and a radio mark; compact cards
    /// lead with an icon. The selected card takes the accent border and a faint accent fill.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn radio_cards(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        value: &str,
        cards: Vec<RadioCard>,
        compact: bool,
        cx: &mut Context<Self>,
        change: impl Fn(&mut Self, &'static str, &mut Context<Self>) + Clone + 'static,
    ) -> AnyElement {
        let id: SharedString = id.into();
        div()
            .id(SharedString::from(format!("radio-cards-{id}")))
            .role(Role::RadioGroup)
            .aria_label(label.into())
            .grid()
            .grid_cols(cards.len() as u16)
            .gap_2()
            .children(cards.into_iter().map(|card| {
                let selected = value == card.value;
                let option = card.value;
                let change = change.clone();
                let title = div()
                    .text_size(px(13.))
                    .line_height(px(19.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(content(1.))
                    .child(card.title.clone());
                let description = div()
                    .text_size(px(12.))
                    .line_height(px(16.))
                    .text_color(content(0.45))
                    .child(card.description.clone());
                div()
                    .id(SharedString::from(format!("radio-card-{id}-{option}")))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("settings-{id}-{option}")
                    })
                    .role(Role::RadioButton)
                    .aria_label(card.title.clone())
                    .aria_selected(selected)
                    // Tab reaches each card, and Enter or Space picks it: GPUI turns either
                    // key into a click on the focused element.
                    .tab_index(0)
                    .focus_visible(|s| s.border_color(content(0.6)))
                    .relative()
                    .min_w_0()
                    .rounded_md()
                    .border_1()
                    .cursor_pointer()
                    .map(|d| {
                        if selected {
                            d.border_color(accent_color()).bg(accent(0.06))
                        } else {
                            d.border_color(content(0.1))
                                .hover(|s| s.border_color(content(0.2)))
                        }
                    })
                    .map(|d| {
                        if compact {
                            d.flex()
                                .items_start()
                                .gap_2()
                                .px(px(10.))
                                .py_2()
                                .when_some(card.glyph, |d, glyph| {
                                    d.child(icon(glyph, 16.).mt(px(2.)).text_color(if selected {
                                        accent_color()
                                    } else {
                                        content(0.6)
                                    }))
                                })
                                .child(div().min_w_0().flex_1().child(title).child(description))
                        } else {
                            d.p_2()
                                .when_some(card.preview, |d, preview| {
                                    d.child(
                                        div()
                                            .h(px(56.))
                                            .mb_2()
                                            .overflow_hidden()
                                            .rounded(px(4.))
                                            .bg(content(0.05))
                                            .p_2()
                                            .child(preview),
                                    )
                                })
                                .child(title)
                                .child(description)
                                // The radio mark of `modal::radio_row`.
                                .child(
                                    div()
                                        .absolute()
                                        .top(px(8.))
                                        .right(px(8.))
                                        .size(px(14.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .border_1()
                                        .border_color(if selected {
                                            accent_color()
                                        } else {
                                            content(0.3)
                                        })
                                        .bg(shell_fill())
                                        .when(selected, |d| {
                                            d.child(
                                                div()
                                                    .size(px(6.))
                                                    .rounded_full()
                                                    .bg(accent_color()),
                                            )
                                        }),
                                )
                        }
                    })
                    .on_click(cx.listener(move |this, _, _, cx| change(this, option, cx)))
            }))
            .into_any_element()
    }

    /// A ladder of levels, lowest first, after a "Default" step: each step shows as many lit
    /// bars as its rank. Used for reasoning effort. The frame and selection are `segmented`'s.
    pub(super) fn level_picker(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        value: Option<&str>,
        levels: &[(String, SharedString)],
        cx: &mut Context<Self>,
        change: impl Fn(&mut Self, Option<String>, &mut Context<Self>) + Clone + 'static,
    ) -> AnyElement {
        let id: SharedString = id.into();
        let count = levels.len();
        let steps = std::iter::once((None, SharedString::from("Default"), 0)).chain(
            levels
                .iter()
                .enumerate()
                .map(|(ix, (level, name))| (Some(level.clone()), name.clone(), ix + 1)),
        );
        div()
            .id(SharedString::from(format!("levels-{id}")))
            .role(Role::RadioGroup)
            .aria_label(label.into())
            .flex()
            .flex_wrap()
            .flex_shrink_0()
            .gap(px(2.))
            .rounded_md()
            .border_1()
            .border_color(content(0.1))
            .p(px(2.))
            .text_size(px(12.))
            .line_height(px(16.))
            .children(steps.map(|(level, name, rank)| {
                let selected = value == level.as_deref();
                let change = change.clone();
                let key = level.clone().unwrap_or_else(|| "default".into());
                let bars = div().h(px(10.)).flex().items_end().gap(px(2.)).map(|d| {
                    if rank == 0 {
                        d.child(icon("minus", 10.).text_color(content(0.5)))
                    } else {
                        d.children((0..count).map(|bar| {
                            div()
                                .w(px(3.))
                                .h(px(4. + bar as f32 * 1.5))
                                .rounded(px(1.))
                                .bg(if bar < rank {
                                    accent_color()
                                } else {
                                    content(0.2)
                                })
                        }))
                    }
                });
                div()
                    .id(SharedString::from(format!("level-{id}-{key}")))
                    .debug_selector({
                        let id = id.clone();
                        move || format!("settings-{id}-{key}")
                    })
                    .role(Role::RadioButton)
                    .aria_label(name.clone())
                    .aria_selected(selected)
                    // Focusable like a radio card: Tab, then Enter or Space.
                    .tab_index(0)
                    .focus_visible(|s| s.bg(content(0.14)).text_color(content(1.)))
                    .min_w(px(56.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_1()
                    .rounded(px(5.))
                    .px_2()
                    .pt(px(6.))
                    .pb_1()
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .map(|d| {
                        if selected {
                            d.bg(content(0.1)).text_color(content(1.))
                        } else {
                            d.text_color(content(0.5))
                                .hover(|s| s.text_color(content(1.)))
                        }
                    })
                    .child(bars)
                    .child(name)
                    .on_click(cx.listener(move |this, _, _, cx| change(this, level.clone(), cx)))
            }))
            .into_any_element()
    }
}
