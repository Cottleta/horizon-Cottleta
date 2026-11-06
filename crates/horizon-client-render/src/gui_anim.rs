//! Cottleta's Settings' GUI animations: one way to time and ease every animated part of the
//! interface, so each looks the same and all of them switch off together. Off by default: an
//! animation off draws its end state at once, exactly as the game does without this.
use std::time::{Duration, Instant};

/// How an animation moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    /// No animation: the end state at once.
    #[default]
    Off,
    /// A short, plain ease out.
    Subtle,
    /// A longer, softer ease in and out.
    Smooth,
    /// Past the end a little, then back.
    Bouncy,
}

impl Style {
    pub const ALL: [Style; 4] = [Style::Off, Style::Subtle, Style::Smooth, Style::Bouncy];

    pub fn name(self) -> &'static str {
        match self {
            Style::Off => "Off",
            Style::Subtle => "Subtle",
            Style::Smooth => "Smooth",
            Style::Bouncy => "Bouncy",
        }
    }
    /// The saved name's style; `Off` for anything else.
    pub fn parse(name: &str) -> Self {
        Style::ALL
            .into_iter()
            .find(|s| s.name().eq_ignore_ascii_case(name))
            .unwrap_or_default()
    }
    /// The next style, round to the first.
    pub fn next(self) -> Self {
        let i = Style::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Style::ALL[(i + 1) % Style::ALL.len()]
    }
    /// How long it lasts at speed 1.
    fn base(self) -> Duration {
        match self {
            Style::Off => Duration::ZERO,
            Style::Subtle => Duration::from_millis(120),
            Style::Smooth => Duration::from_millis(220),
            Style::Bouncy => Duration::from_millis(320),
        }
    }
    /// How long it lasts at `speed` (1 normal, 2 twice as fast; kept between 0.25 and 4).
    pub fn duration(self, speed: f32) -> Duration {
        self.base().div_f32(speed.clamp(0.25, 4.0))
    }
    /// Progress `t` (0 to 1) eased: 0 at the start, 1 at the end. `Bouncy` passes 1 on the way.
    pub fn ease(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Style::Off => 1.0,
            Style::Subtle => 1.0 - (1.0 - t).powi(2),
            Style::Smooth => t * t * (3.0 - 2.0 * t),
            Style::Bouncy => {
                // easeOutBack.
                const C1: f32 = 1.70158;
                const C3: f32 = C1 + 1.0;
                let u = t - 1.0;
                1.0 + C3 * u.powi(3) + C1 * u.powi(2)
            }
        }
    }
}

/// One animation's settings: its style and speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setting {
    pub style: Style,
    pub speed: f32,
}

impl Default for Setting {
    fn default() -> Self {
        Self {
            style: Style::Off,
            speed: 1.0,
        }
    }
}

impl Setting {
    /// The saved form, `style:speed`.
    pub fn to_text(self) -> String {
        format!("{}:{}", self.style.name().to_lowercase(), self.speed)
    }
    /// From the saved form; the default for anything it can't read.
    pub fn parse(text: &str) -> Self {
        let (style, speed) = text.split_once(':').unwrap_or((text, "1"));
        Self {
            style: Style::parse(style),
            speed: speed.parse::<f32>().ok().filter(|s| s.is_finite()).unwrap_or(1.0).clamp(0.25, 4.0),
        }
    }
    pub fn on(self) -> bool {
        self.style != Style::Off
    }
}

/// A change from one value to another, started at a moment: where it is now.
#[derive(Clone, Copy, Debug)]
pub struct Tween {
    from: f32,
    to: f32,
    started: Instant,
    setting: Setting,
}

impl Tween {
    /// At `value`, not moving.
    pub fn at(value: f32) -> Self {
        Self {
            from: value,
            to: value,
            started: Instant::now(),
            setting: Setting::default(),
        }
    }
    /// Heads for `to` from where it is now, eased by `setting`. Off: there at once.
    pub fn go(&mut self, to: f32, setting: Setting, now: Instant) {
        if to == self.to {
            return;
        }
        self.from = self.value(now);
        self.to = to;
        self.started = now;
        self.setting = setting;
    }
    /// Where it is at `now`.
    pub fn value(&self, now: Instant) -> f32 {
        let length = self.setting.style.duration(self.setting.speed);
        if length.is_zero() {
            return self.to;
        }
        let t = now.saturating_duration_since(self.started).as_secs_f32() / length.as_secs_f32();
        self.from + (self.to - self.from) * self.setting.style.ease(t)
    }
    /// Whether it is still moving at `now`: a frame that draws it can't be reused.
    pub fn moving(&self, now: Instant) -> bool {
        let length = self.setting.style.duration(self.setting.speed);
        !length.is_zero() && now.saturating_duration_since(self.started) < length
    }
    pub fn target(&self) -> f32 {
        self.to
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_style_starts_at_zero_and_ends_at_one() {
        for style in Style::ALL {
            if style != Style::Off {
                assert!(style.ease(0.0).abs() < 1e-6, "{style:?}");
            }
            assert!((style.ease(1.0) - 1.0).abs() < 1e-6, "{style:?}");
        }
    }

    #[test]
    fn bouncy_goes_past_the_end_and_the_others_do_not() {
        let peak = |s: Style| (0..=100).map(|i| s.ease(i as f32 / 100.0)).fold(0.0, f32::max);
        assert!(peak(Style::Bouncy) > 1.05);
        assert!(peak(Style::Subtle) <= 1.0 + 1e-6 && peak(Style::Smooth) <= 1.0 + 1e-6);
    }

    #[test]
    fn off_is_there_at_once_and_never_moving() {
        let start = Instant::now();
        let mut tween = Tween::at(0.0);
        tween.go(1.0, Setting::default(), start);
        assert_eq!(tween.value(start), 1.0);
        assert!(!tween.moving(start));
    }

    #[test]
    fn a_tween_eases_to_its_target_and_turns_from_where_it_is() {
        let start = Instant::now();
        let smooth = Setting {
            style: Style::Smooth,
            speed: 1.0,
        };
        let mut tween = Tween::at(0.0);
        tween.go(10.0, smooth, start);
        let half = start + Style::Smooth.duration(1.0) / 2;
        assert!((tween.value(half) - 5.0).abs() < 0.01);
        assert!(tween.moving(half));
        // Turned back halfway: from 5, not from 10.
        tween.go(0.0, smooth, half);
        assert!((tween.value(half) - 5.0).abs() < 0.01);
        let end = half + Style::Smooth.duration(1.0);
        assert_eq!(tween.value(end), 0.0);
        assert!(!tween.moving(end));
    }

    #[test]
    fn settings_save_and_read_back_and_bad_text_is_off() {
        let s = Setting {
            style: Style::Bouncy,
            speed: 1.5,
        };
        assert_eq!(Setting::parse(&s.to_text()), s);
        assert_eq!(Setting::parse("nonsense"), Setting::default());
        assert_eq!(Setting::parse("smooth:99").speed, 4.0);
        assert_eq!(Style::Smooth.duration(2.0), Duration::from_millis(110));
    }
}
