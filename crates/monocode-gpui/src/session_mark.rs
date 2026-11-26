//! A session's provider mark, working spinner, circled done check and amber
//! attention mark, for sidebar rows and title tabs.
use crate::pulse::{self, Rate};
use crate::theme::*;
use gpui::{
    AnyElement, Context, ElementId, Hsla, Render, Role, Svg, Window, div, prelude::*, px, rgb,
};
use monocode_core::agent_status::{Activity, Outcome, Wait};
use std::f32::consts::TAU;
use std::time::Duration;

/// One wave across the grid, as TerminalSpinner's `monocode-spinner-wave`.
pub(crate) const SPINNER_PERIOD: Duration = Duration::from_millis(1_200);
/// Slate, peach and pink, top row to bottom.
pub(crate) const SPINNER_ROWS: [u32; 3] = [0x6f84a3, 0xe8b583, 0xe47fa6];

/// The working spinner turning on the shared clock at `rate`.
#[track_caller]
pub(crate) fn spinner(id: impl Into<ElementId>, rate: Rate) -> pulse::Looping {
    pulse::looping(id, SPINNER_PERIOD, rate, spinner_at)
}

/// TerminalSpinner's 12px 3×3 grid at `fraction` of its period: a soft wave
/// washes from the top-left corner to the bottom-right, and dots off the
/// center diagonal peak dimmer.
pub(crate) fn spinner_at(fraction: f32) -> gpui::Div {
    let phase = fraction * TAU;
    div()
        .size(px(12.))
        .flex()
        .flex_col()
        .children((0..3).map(|row| {
            div().flex().children((0..3).map(move |col| {
                let wave = (0.5 - 0.5 * (phase - (col + row) as f32 * 0.9).cos()).powf(3.52);
                let peak = 0.93 * (1. - 0.65 * (col as f32 - row as f32).abs() / 2.);
                let alpha = 0.07 + peak * wave;
                let mut color: Hsla = rgb(SPINNER_ROWS[row]).into();
                color.a = alpha;
                div()
                    .size(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .size(px(2.5 * (0.89 + 0.11 * peak * wave)))
                            .rounded_full()
                            .bg(color),
                    )
            }))
        }))
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    Idle,
    Working,
    Waiting,
    Stale,
    Done,
    /// Waiting on an approval or a question.
    Attention,
}
impl State {
    pub(crate) fn from_activity(activity: &Activity) -> Self {
        match activity {
            Activity::NeedsYou { .. } => Self::Attention,
            Activity::Running { stale: true, .. } => Self::Stale,
            Activity::Running { .. } => Self::Working,
            Activity::BackgroundWait { .. } => Self::Waiting,
            Activity::Done { unread: true, .. } => Self::Done,
            _ => Self::Idle,
        }
    }
}

pub(crate) fn activity_color(activity: &Activity) -> Hsla {
    match activity {
        Activity::NeedsYou { .. }
        | Activity::BackgroundWait {
            reason: Wait::UsageLimit { .. } | Wait::UsageLimitPending,
            ..
        } => warning_color(),
        Activity::Running { stale: false, .. } | Activity::BackgroundWait { .. } => accent_color(),
        Activity::Done {
            outcome: Outcome::Completed,
            unread: true,
            ..
        } => success_color(),
        Activity::Done {
            outcome: Outcome::Failed,
            unread: true,
            ..
        } => danger_color(),
        _ => content(0.55),
    }
}

pub(crate) fn activity_glyph(activity: &Activity, side: f32) -> Option<Svg> {
    let name = match activity {
        Activity::NeedsYou { .. } => "alert-circle",
        Activity::Running { stale: true, .. } => "clock",
        Activity::BackgroundWait {
            reason: Wait::Subtasks(_),
            ..
        } => "queue-pause",
        Activity::BackgroundWait { .. } => "clock",
        Activity::Done {
            outcome,
            unread: true,
            ..
        } => match outcome {
            Outcome::Completed => "check",
            Outcome::Failed => "circle-x",
            Outcome::Interrupted => "square",
        },
        _ => return None,
    };
    Some(icon(name, side).text_color(activity_color(activity)))
}

/// The two placements use the same artwork and different ring geometry.
#[derive(Clone, Copy)]
pub(crate) enum ActivityIconSurface {
    Tab,
    Sidebar,
}
impl ActivityIconSurface {
    fn tile(self) -> &'static crate::tab_tile::TileSize {
        match self {
            Self::Tab => &crate::tab_tile::TAB_TILE,
            Self::Sidebar => &crate::tab_tile::SIDEBAR_TILE,
        }
    }
}

/// How much of the corner badge is drawn, for rows easing it in or out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BadgeLook {
    /// Scale about the badge's centre; may overshoot 1 while popping in.
    pub(crate) scale: f32,
    pub(crate) opacity: f32,
}
impl BadgeLook {
    pub(crate) const FULL: Self = Self {
        scale: 1.,
        opacity: 1.,
    };
}

/// The provider tile with its activity mark, the badge drawn at `badge`
/// ([`BadgeLook::FULL`] at rest).
pub(crate) fn provider_activity_icon(
    provider: &str,
    activity: &Activity,
    surface: ActivityIconSurface,
    edge: Hsla,
    reduce_motion: bool,
    badge: BadgeLook,
) -> AnyElement {
    let (face, mark) =
        activity_tile_parts_with_badge(provider, activity, surface, edge, reduce_motion, badge);
    crate::tab_tile::mark_tile(surface.tile(), face, mark).into_any_element()
}

/// A mark can also fit inside the existing tab stack and pinned deck.
pub(crate) fn activity_tile_parts(
    provider: &str,
    activity: &Activity,
    surface: ActivityIconSurface,
    edge: Hsla,
    reduce_motion: bool,
) -> (crate::tab_tile::Face, AnyElement) {
    activity_tile_parts_with_badge(
        provider,
        activity,
        surface,
        edge,
        reduce_motion,
        BadgeLook::FULL,
    )
}

fn activity_tile_parts_with_badge(
    provider: &str,
    activity: &Activity,
    surface: ActivityIconSurface,
    edge: Hsla,
    reduce_motion: bool,
    look: BadgeLook,
) -> (crate::tab_tile::Face, AnyElement) {
    let (side, badge_side, border, ring_width) = match surface {
        ActivityIconSurface::Tab => (14., 8., 2., 1.5),
        ActivityIconSurface::Sidebar => (10., 7., 1.5, 1.25),
    };
    let mut face = crate::tab_tile::Face::provider(provider);
    let badge = match activity {
        Activity::NeedsYou { .. } => Some((warning_color(), false)),
        Activity::BackgroundWait { .. } => Some((agent_running_color(), true)),
        Activity::Done { unread: true, .. } => Some((agent_unread_color(), false)),
        _ => None,
    }
    .filter(|_| look.scale > 0. && look.opacity > 0.);
    // The badge shrinks towards its own centre, staying pinned to the corner.
    let outer = badge_side + border * 2.;
    let inset = -2. + outer * (1. - look.scale) / 2.;
    let ring = activity.is_animating().then(|| {
        face = face.flat();
        let radius = surface.tile().radius;
        let color = agent_running_color();
        if reduce_motion {
            running_outline(radius, ring_width, color, 0.48)
        } else {
            pulse::looping(
                "agent-tile-running",
                Duration::from_millis(1600),
                Rate::Half,
                move |phase| running_outline(radius, ring_width, color, phase),
            )
            .into_any_element()
        }
    });
    let mark = div()
        .relative()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(provider_glyph(provider, side))
        .children(ring)
        .when_some(badge, |d, (color, hollow)| {
            d.child(
                div()
                    .absolute()
                    .right(px(inset))
                    .bottom(px(inset))
                    .size(px(outer * look.scale))
                    .rounded_full()
                    .bg(edge)
                    .opacity(look.opacity)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .size(px(badge_side * look.scale))
                            .rounded_full()
                            .when(hollow, |d| d.border_1().border_color(color))
                            .when(!hollow, |d| d.bg(color)),
                    ),
            )
        });
    (face, mark.into_any_element())
}

fn running_outline(radius: f32, width: f32, color: Hsla, phase: f32) -> AnyElement {
    use gpui::{BorderStyle, Bounds, Corners, Edges, PathBuilder, canvas, point, size};

    let outset = 1. + width;
    let outer_radius = radius + outset;
    let path_radius = outer_radius - width / 2.;
    canvas(
        |_, _, _| {},
        move |tile, _, window, _| {
            let extend = point(px(outset), px(outset));
            let track = Bounds::new(
                tile.origin - extend,
                tile.size + size(px(2. * outset), px(2. * outset)),
            );
            window.paint_quad(
                gpui::outline(track, color.opacity(0.24), BorderStyle::Solid)
                    .border_widths(Edges::all(px(width)))
                    .corner_radii(Corners::all(px(outer_radius))),
            );

            // Measure the sweep along the rounded perimeter to keep its speed
            // constant through the corners and along the longer horizontal edges.
            let inset = point(px(width / 2.), px(width / 2.));
            let origin = track.origin + inset;
            let w = track.size.width.as_f32() - width;
            let h = track.size.height.as_f32() - width;
            const STEPS: usize = 18;
            for i in 0..STEPS {
                let blend = (i + 1) as f32 / STEPS as f32;
                let t = phase - (1. - blend) * 0.33;
                let sweep = 0.33 / STEPS as f32;
                let locate = |at| {
                    let (x, y) = contour_point(w, h, path_radius, at);
                    origin + point(px(x), px(y))
                };
                let mut stroke = PathBuilder::stroke(px(width));
                stroke.move_to(locate(t - sweep));
                stroke.line_to(locate(t + sweep * 0.12));
                if let Ok(path) = stroke.build() {
                    let tint = color.blend(white(0.48 * blend.powi(6)));
                    window.paint_path(path, tint.opacity(0.87 * blend.powi(2)));
                }
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}

fn contour_point(w: f32, h: f32, radius: f32, phase: f32) -> (f32, f32) {
    let horizontal = w - 2. * radius;
    let vertical = h - 2. * radius;
    let quarter = radius * std::f32::consts::FRAC_PI_2;
    let perimeter = 2. * (horizontal + vertical) + 4. * quarter;
    let mut distance = phase.rem_euclid(1.) * perimeter;
    for (index, (x, y)) in [
        (w - radius, radius),
        (w - radius, h - radius),
        (radius, h - radius),
        (radius, radius),
    ]
    .into_iter()
    .enumerate()
    {
        let side = if index % 2 == 0 { horizontal } else { vertical };
        if distance < side {
            return match index {
                0 => (radius + distance, 0.),
                1 => (w, radius + distance),
                2 => (w - radius - distance, h),
                _ => (0., h - radius - distance),
            };
        }
        distance -= side;
        if distance < quarter {
            let angle = (index as f32 - 1.) * std::f32::consts::FRAC_PI_2 + distance / radius;
            return (x + angle.cos() * radius, y + angle.sin() * radius);
        }
        distance -= quarter;
    }
    (radius, 0.)
}

/// A small cached view so animation leaves the conversation untouched.
pub(crate) struct SessionMark {
    side: f32,
    provider: String,
    state: State,
}
impl SessionMark {
    pub(crate) fn new(provider: String) -> Self {
        Self {
            side: 16.,
            provider,
            state: State::Idle,
        }
    }
    /// A title tab's mark. Tabs paint inside their own cached frame, so they
    /// can't share the sidebar's entity: a notify only repaints the cached
    /// views along the one path the entity was last drawn in.
    pub(crate) fn tab(provider: String) -> Self {
        Self {
            side: 14.,
            ..Self::new(provider)
        }
    }
    #[cfg(test)]
    pub(crate) fn state(&self) -> State {
        self.state
    }
    pub(crate) fn set_size(&mut self, side: f32, cx: &mut Context<Self>) {
        if self.side != side {
            self.side = side;
            cx.notify();
        }
    }
    pub(crate) fn set(&mut self, provider: &str, state: State, cx: &mut Context<Self>) {
        if self.state == state && self.provider == provider {
            return;
        }
        self.provider = provider.to_owned();
        self.state = state;
        cx.notify();
    }
}
/// The mark for `state` at `side`: the provider's icon when idle, else the
/// working spinner, a waiting or stale mark, the done check or the approval alert. Shared by the
/// sidebar and tab marks and any row that shows a session's state.
pub(crate) fn mark_element(provider: &str, state: State, side: f32) -> AnyElement {
    match state {
        State::Idle => provider_icon(provider, side).into_any_element(),
        // Its own small view, so the clock repaints only the mark.
        State::Working => div()
            .id("session-working")
            .child(spinner("session-working-spinner", Rate::Smooth))
            .role(Role::Image)
            .aria_label("Working")
            .into_any_element(),
        State::Waiting => div()
            .id("session-waiting")
            .role(Role::Image)
            .aria_label("Waiting in background")
            .child(icon("queue-pause", side).text_color(accent_color()))
            .into_any_element(),
        State::Stale => div()
            .id("session-stale")
            .role(Role::Image)
            .aria_label("No update in 30 minutes")
            .child(icon("clock", side).text_color(content(0.55)))
            .into_any_element(),
        // Tailwind teal-400 and amber-400, as TitleBar.tsx.
        State::Done => div()
            .id("session-done")
            .role(Role::Image)
            .aria_label("Done")
            .child(icon("check-circle-2", 14.).text_color(rgb(0x2dd4bf)))
            .into_any_element(),
        State::Attention => div()
            .id("session-attention")
            .role(Role::Image)
            .aria_label("Needs approval")
            .child(icon("alert-circle", 14.).text_color(rgb(0xfbbf24)))
            .into_any_element(),
    }
}
impl Render for SessionMark {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let side = self.side;
        let mark = mark_element(&self.provider, self.state, side);
        div()
            .debug_selector(|| "session-provider-mark".into())
            .size(px(side))
            .flex()
            .items_center()
            .justify_center()
            .child(mark)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    #[gpui::test]
    fn mark_spins_only_while_the_agent_is_actively_running(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(false));
        let (mark, cx) = cx.add_window_view(|_, _| SessionMark::new("codex".into()));
        for quiet in [State::Waiting, State::Stale, State::Done] {
            mark.update(cx, |mark, cx| mark.set("codex", State::Working, cx));
            cx.run_until_parked();
            cx.update(|_, cx| assert!(pulse::running(cx)));
            mark.update(cx, |mark, cx| mark.set("codex", quiet, cx));
            cx.run_until_parked();
            // Two background-paced strides: one notifies, the next finds no loop.
            cx.executor().advance_clock(Duration::from_secs(2));
            cx.run_until_parked();
            cx.update(|_, cx| assert!(!pulse::running(cx)));
        }
    }

    #[test]
    fn running_outline_keeps_a_uniform_gap_and_loops_at_the_same_point() {
        for (w, h, radius) in [(27.5, 23.5, 6.75), (19.25, 17.25, 5.625)] {
            let start = contour_point(w, h, radius, 0.);
            let opposite = contour_point(w, h, radius, 0.5);
            assert_eq!(start, (radius, 0.));
            assert_eq!(contour_point(w, h, radius, 1.), start);
            assert!((opposite.0 - (w - radius)).abs() < 0.001);
            assert!((opposite.1 - h).abs() < 0.001);
            for step in 0..100 {
                let (x, y) = contour_point(w, h, radius, step as f32 / 100.);
                assert!((0. ..=w).contains(&x));
                assert!((0. ..=h).contains(&y));
            }
        }
    }
}
