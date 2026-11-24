//! A disclosure: content under a header that opens and closes by animating the height it
//! is clipped to, so what sits below moves with it instead of jumping.
//!
//! [`Disclosure`] holds the state of one such region. The owner calls [`Disclosure::set`]
//! when it opens or closes, draws [`Disclosure::render`] while [`Disclosure::mounted`], and
//! asks for another frame while [`Disclosure::animating`]. The content is laid out at its
//! own height and only clipped, never squashed; that height is measured as it is laid out,
//! so content that grows or shrinks while open is simply followed.
use crate::popover::AnchorSlot;
use gpui::{Div, div, prelude::*, px};
use std::time::{Duration, Instant};

/// How long opening or closing takes: the modal card's entrance time.
pub(crate) const DURATION: Duration = Duration::from_millis(200);

/// The modal card's ease-out.
fn ease_out(t: f32) -> f32 {
    1. - (1. - t).powi(3)
}

#[derive(Clone)]
pub(crate) struct Disclosure {
    open: bool,
    /// How far open it was when the running animation began, so a toggle in the middle of
    /// one continues from where the content is instead of jumping to an end.
    from: f32,
    started: Option<Instant>,
    /// The content's own bounds, written as it is laid out.
    content: AnchorSlot,
}

impl Disclosure {
    pub(crate) fn new(open: bool) -> Self {
        Self {
            open,
            from: if open { 1. } else { 0. },
            started: None,
            content: AnchorSlot::default(),
        }
    }

    /// Open or close. Without `animate` (reduced motion) it is at its end at once.
    pub(crate) fn set(&mut self, open: bool, animate: bool, now: Instant) {
        if self.open == open {
            return;
        }
        self.from = self.fraction(now);
        self.open = open;
        self.started = animate.then_some(now);
    }

    fn target(&self) -> f32 {
        if self.open { 1. } else { 0. }
    }

    /// How far open it is, from 0 (closed) to 1 (open), eased.
    pub(crate) fn fraction(&self, now: Instant) -> f32 {
        let Some(started) = self.started else {
            return self.target();
        };
        let t = now.saturating_duration_since(started).as_secs_f32() / DURATION.as_secs_f32();
        if t >= 1. {
            return self.target();
        }
        self.from + (self.target() - self.from) * ease_out(t)
    }

    /// An open or close is under way: the owner keeps drawing frames.
    pub(crate) fn animating(&self, now: Instant) -> bool {
        self.started
            .is_some_and(|started| now.saturating_duration_since(started) < DURATION)
    }

    /// The content is on screen: open, or still closing.
    pub(crate) fn mounted(&self, now: Instant) -> bool {
        self.open || self.animating(now)
    }

    /// The content in its clip, or nothing once it has closed. At rest the clip takes the
    /// content's height; while animating it is that height times how far open it is, and
    /// the content fades with it.
    pub(crate) fn render(&self, content: impl IntoElement, now: Instant) -> Option<Div> {
        if !self.mounted(now) {
            return None;
        }
        let probe = div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(self.content.layout_probe());
        // Not shrinking keeps the content at its own height inside a shorter clip.
        let inner = div().relative().flex_shrink_0().child(probe).child(content);
        let clip = div().flex().flex_col().overflow_hidden();
        let clip = if self.animating(now) {
            let fraction = self.fraction(now);
            // Until the content has been laid out once its height is unknown: the first
            // frame of an opening stays shut and the next one has the measure.
            let height = self
                .content
                .get()
                .map_or(px(0.), |bounds| bounds.size.height * fraction);
            clip.h(height).opacity(fraction)
        } else {
            clip
        };
        Some(clip.child(inner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    #[test]
    fn opening_eases_from_shut_to_open_and_then_rests() {
        let start = Instant::now();
        let mut disclosure = Disclosure::new(false);
        assert!(!disclosure.mounted(start));
        assert_eq!(disclosure.fraction(start), 0.);
        disclosure.set(true, true, start);
        assert!(disclosure.mounted(start));
        assert!(disclosure.animating(start));
        assert_eq!(disclosure.fraction(start), 0.);
        let middle = disclosure.fraction(at(start, 100));
        // Ease-out: more than half the way at half the time, and never past the end.
        assert!(middle > 0.5 && middle < 1., "{middle}");
        assert!(disclosure.fraction(at(start, 150)) > middle);
        let end = at(start, 200);
        assert_eq!(disclosure.fraction(end), 1.);
        assert!(!disclosure.animating(end));
        assert!(disclosure.mounted(end));
    }

    #[test]
    fn closing_keeps_the_content_mounted_until_it_has_shut() {
        let start = Instant::now();
        let mut disclosure = Disclosure::new(true);
        assert_eq!(disclosure.fraction(start), 1.);
        disclosure.set(false, true, start);
        assert!(disclosure.mounted(at(start, 199)));
        let middle = disclosure.fraction(at(start, 100));
        assert!(middle > 0. && middle < 0.5, "{middle}");
        assert_eq!(disclosure.fraction(at(start, 200)), 0.);
        assert!(!disclosure.mounted(at(start, 200)));
    }

    #[test]
    fn a_toggle_in_the_middle_continues_from_where_it_is() {
        let start = Instant::now();
        let mut disclosure = Disclosure::new(false);
        disclosure.set(true, true, start);
        let turn = at(start, 60);
        let reached = disclosure.fraction(turn);
        disclosure.set(false, true, turn);
        // No jump at the turn, then it eases back to shut over a full duration.
        assert_eq!(disclosure.fraction(turn), reached);
        assert!(disclosure.fraction(at(start, 120)) < reached);
        assert!(disclosure.mounted(at(start, 259)));
        assert_eq!(disclosure.fraction(at(start, 260)), 0.);
        // Setting what it already is changes nothing.
        let again = at(start, 100);
        let before = disclosure.fraction(again);
        disclosure.set(false, true, again);
        assert_eq!(disclosure.fraction(again), before);
    }

    #[test]
    fn reduced_motion_is_at_its_end_at_once() {
        let start = Instant::now();
        let mut disclosure = Disclosure::new(false);
        disclosure.set(true, false, start);
        assert_eq!(disclosure.fraction(start), 1.);
        assert!(!disclosure.animating(start));
        disclosure.set(false, false, start);
        assert_eq!(disclosure.fraction(start), 0.);
        assert!(!disclosure.mounted(start));
    }
}
