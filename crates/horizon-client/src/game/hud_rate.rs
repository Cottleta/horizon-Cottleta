//! Dynamic HUD refresh (the `dynamicHud` graphics option): while nothing is touched
//! the HUD is rebuilt 20 times a second, as a phone's display slows when left alone,
//! and a key, a click or the wheel brings it back up for a moment. Between rebuilds
//! the frame draws the last one again. Turning the view changes nothing in it but a
//! locator bar's marks, so the mouse alone counts only for those, at 60 times a second.
use super::{Game, Screen};
use horizon_client::hud::HudBatch;
use std::time::{Duration, Instant};

/// How long a rebuild lasts while nothing was touched.
const IDLE: Duration = Duration::from_millis(50);
/// How long one lasts just after a touch: 240 times a second.
const TOUCHED: Duration = Duration::from_micros(4167);
/// How long one lasts while the view turns with a locator bar on screen.
const TURNING: Duration = Duration::from_micros(16667);
/// How long a touch keeps the fast rate.
const TOUCH_HOLD: Duration = Duration::from_secs(1);

/// How long a rebuild lasts, this long after the last touch.
fn lasts(since_touch: Duration, marks_turning: bool) -> Duration {
    if since_touch < TOUCH_HOLD {
        TOUCHED
    } else if marks_turning {
        TURNING
    } else {
        IDLE
    }
}

impl Game<'_> {
    /// Whether the HUD may stay as it was: plain play, with no screen, the debug overlay
    /// or a native mod drawing into it.
    fn hud_may_rest(&self) -> bool {
        self.settings.horizon.dynamic_hud
            && self.title.is_none()
            && self.screen == Screen::Playing
            && !self.pause
            && !self.debug
            && self.sign_edit.is_none()
            && self.book.is_none()
            && !self.dialog_open()
            && horizon_client::mod_api::mods().is_empty()
            && !self.screen_anim.moving(Instant::now())
            && !self.hotbar_slide.moving(Instant::now())
    }
    /// A key, a click or the wheel: the next frame shows what it changed.
    pub(super) fn touch_hud(&mut self) {
        self.hud_touched = Instant::now();
        self.hud_kept = None;
    }
    /// This frame's HUD: the kept one while it lasts, else a new one.
    pub(super) fn frame_hud(&mut self, size: [u32; 2]) -> HudBatch {
        let kept = self.hud_kept.take();
        let view = (self.yaw, self.pitch);
        let marks_turning = view != self.hud_view && self.waypoints.has_waypoints();
        if self.hud_may_rest()
            && let Some((kept_size, mut hud)) = kept
            && kept_size == size
            && self.hud_built.elapsed() < lasts(self.hud_touched.elapsed(), marks_turning)
        {
            // Its glyphs went up with the frame that built it.
            hud.patches.clear();
            return hud;
        }
        self.hud_built = Instant::now();
        self.hud_view = view;
        horizon_client::spans::time("hud", || self.hud(size))
    }
    /// Keeps the frame's HUD for the next ones, when they may use it.
    pub(super) fn keep_hud(&mut self, size: [u32; 2], hud: HudBatch) {
        self.hud_kept = self.hud_may_rest().then_some((size, hud));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_touch_speeds_the_hud_up_for_a_moment() {
        assert_eq!(lasts(Duration::ZERO, false), TOUCHED);
        assert_eq!(lasts(Duration::from_millis(999), true), TOUCHED);
        assert_eq!(lasts(TOUCH_HOLD, false), IDLE);
        assert_eq!(lasts(TOUCH_HOLD, true), TURNING);
        assert!(TOUCHED < TURNING && TURNING < IDLE);
    }
}
