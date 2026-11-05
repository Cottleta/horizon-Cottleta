//! A third swapchain image while frames are short. The display holds on to images it shows, and at
//! thousands of frames a second the game then waits for one, about each refresh: a spare image
//! takes most of those waits away, for the 1% lows. Its cost is the CPU getting one frame further
//! ahead of a busy GPU, so it is only there while one frame is too short to feel.
use std::time::Duration;

/// How often the frame rate is read.
const WINDOW: Duration = Duration::from_secs(1);
/// Frames a window takes the spare image at: one more frame ahead costs at most a millisecond.
const FAST: u32 = 1000;
/// Frames a window gives it back below.
const SLOW: u32 = 700;
/// Windows in a row above `FAST` before taking it: building a swapchain is a hitch of its own.
const FAST_WINDOWS: u32 = 3;

#[derive(Default)]
pub(super) struct SpareImage {
    on: bool,
    spent: Duration,
    frames: u32,
    fast_windows: u32,
}

impl SpareImage {
    /// One frame of `elapsed`. The new answer when it changes.
    pub(super) fn frame(&mut self, elapsed: Duration) -> Option<bool> {
        self.spent += elapsed;
        self.frames += 1;
        if self.spent < WINDOW {
            return None;
        }
        let per_second = (f64::from(self.frames) / self.spent.as_secs_f64()) as u32;
        self.spent = Duration::ZERO;
        self.frames = 0;
        self.fast_windows = if per_second >= FAST {
            self.fast_windows + 1
        } else {
            0
        };
        let on = if self.on {
            per_second >= SLOW
        } else {
            self.fast_windows >= FAST_WINDOWS
        };
        (on != self.on).then(|| {
            self.on = on;
            on
        })
    }
    /// Without it again (the present mode changed).
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second of frames at `fps`; the last answer.
    fn second(spare: &mut SpareImage, fps: u32) -> Option<bool> {
        let each = Duration::from_secs_f64(1.0 / f64::from(fps));
        (0..=fps).filter_map(|_| spare.frame(each)).last()
    }

    #[test]
    fn it_comes_after_a_few_fast_seconds_and_goes_in_one_slow_one() {
        let mut spare = SpareImage::default();
        assert_eq!(second(&mut spare, 4000), None);
        assert_eq!(second(&mut spare, 4000), None);
        assert_eq!(second(&mut spare, 4000), Some(true));
        assert_eq!(
            second(&mut spare, 800),
            None,
            "between the two rates it stays"
        );
        assert_eq!(second(&mut spare, 300), Some(false));
        assert_eq!(second(&mut spare, 300), None);
    }

    #[test]
    fn a_slow_second_starts_the_count_again() {
        let mut spare = SpareImage::default();
        assert_eq!(second(&mut spare, 4000), None);
        assert_eq!(second(&mut spare, 4000), None);
        assert_eq!(second(&mut spare, 60), None);
        assert_eq!(second(&mut spare, 4000), None);
        assert_eq!(second(&mut spare, 4000), None);
        assert_eq!(second(&mut spare, 4000), Some(true));
    }
}
