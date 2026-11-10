//! Cottleta's Settings' GUI animations: one way to time and ease every animated part of the
//! interface, so each looks the same and all of them switch off together. Off by default: an
//! animation off draws its end state at once, exactly as the game does without this.
use std::time::{Duration, Instant};

/// How an animation's progress is eased (the usual smoothing choices).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    /// No animation: the end state at once.
    #[default]
    Off,
    /// The same pace throughout.
    Linear,
    /// Slow at first, then fast.
    EaseIn,
    /// Fast at first, then slowing to a stop.
    EaseOut,
    /// Slow at both ends.
    EaseInOut,
}

impl Style {
    pub const ALL: [Style; 5] = [
        Style::Off,
        Style::Linear,
        Style::EaseIn,
        Style::EaseOut,
        Style::EaseInOut,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Style::Off => "Off",
            Style::Linear => "Linear",
            Style::EaseIn => "Ease In",
            Style::EaseOut => "Ease Out",
            Style::EaseInOut => "Ease In-Out",
        }
    }
    /// The saved name (`name` without spaces, any case); `Off` for anything else.
    pub fn parse(name: &str) -> Self {
        let key = |s: &str| s.replace([' ', '-'], "").to_lowercase();
        Style::ALL
            .into_iter()
            .find(|s| key(s.name()) == key(name))
            .unwrap_or_default()
    }
    fn key(self) -> String {
        self.name().replace([' ', '-'], "").to_lowercase()
    }
    /// The next style, round to the first.
    pub fn next(self) -> Self {
        let i = Style::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Style::ALL[(i + 1) % Style::ALL.len()]
    }
    /// How long it lasts at `speed` (1 normal, 2 twice as fast; kept between 0.25 and 4).
    pub fn duration(self, speed: f32) -> Duration {
        if self == Style::Off {
            return Duration::ZERO;
        }
        Duration::from_millis(200).div_f32(speed.clamp(0.25, 4.0))
    }
    /// Progress `t` (0 to 1) eased: 0 at the start, 1 at the end.
    pub fn ease(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Style::Off => 1.0,
            Style::Linear => t,
            Style::EaseIn => t * t * t,
            Style::EaseOut => 1.0 - (1.0 - t).powi(3),
            Style::EaseInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
        }
    }
}

/// Where a screen comes from as it opens (and goes as it closes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Motion {
    /// It fades in where it is.
    #[default]
    Fade,
    FromTop,
    FromBottom,
    FromLeft,
    FromRight,
    /// It grows from the middle of the screen, and shrinks back into it as it closes.
    FromMiddle,
}

impl Motion {
    pub const ALL: [Motion; 6] = [
        Motion::Fade,
        Motion::FromTop,
        Motion::FromBottom,
        Motion::FromLeft,
        Motion::FromRight,
        Motion::FromMiddle,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Motion::Fade => "Fade",
            Motion::FromTop => "From Top",
            Motion::FromBottom => "From Bottom",
            Motion::FromLeft => "From Left",
            Motion::FromRight => "From Right",
            Motion::FromMiddle => "From Middle",
        }
    }
    pub fn parse(name: &str) -> Self {
        let key = |s: &str| s.replace(' ', "").to_lowercase();
        Motion::ALL
            .into_iter()
            .find(|m| key(m.name()) == key(name))
            .unwrap_or_default()
    }
    pub fn next(self) -> Self {
        let i = Motion::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Motion::ALL[(i + 1) % Motion::ALL.len()]
    }
    /// At progress `p` (0 hidden, 1 in place), for a screen of `size` (GUI pixels): its offset,
    /// its scale about the screen's middle and its opacity.
    pub fn pose(self, p: f32, size: [f32; 2]) -> ([f32; 2], f32, f32) {
        let away = 1.0 - p;
        let fade = p.clamp(0.0, 1.0);
        match self {
            Motion::Fade => ([0.0, 0.0], 1.0, fade),
            Motion::FromTop => ([0.0, -away * size[1] * 0.5], 1.0, fade),
            Motion::FromBottom => ([0.0, away * size[1] * 0.5], 1.0, fade),
            Motion::FromLeft => ([-away * size[0] * 0.5, 0.0], 1.0, fade),
            Motion::FromRight => ([away * size[0] * 0.5, 0.0], 1.0, fade),
            Motion::FromMiddle => ([0.0, 0.0], p.max(0.0), fade),
        }
    }
}

/// One animation's settings: its easing, where it comes from, and its speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setting {
    pub style: Style,
    pub motion: Motion,
    pub speed: f32,
}

impl Default for Setting {
    fn default() -> Self {
        Self {
            style: Style::Off,
            motion: Motion::Fade,
            speed: 1.0,
        }
    }
}

impl Setting {
    /// The saved form, `style:motion:speed`.
    pub fn to_text(self) -> String {
        format!(
            "{}:{}:{}",
            self.style.key(),
            self.motion.name().replace(' ', "").to_lowercase(),
            self.speed
        )
    }
    /// From the saved form; the default for anything it can't read.
    pub fn parse(text: &str) -> Self {
        let mut parts = text.split(':');
        let style = Style::parse(parts.next().unwrap_or(""));
        let motion = Motion::parse(parts.next().unwrap_or(""));
        let speed = parts
            .next()
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|s| s.is_finite())
            .unwrap_or(1.0)
            .clamp(0.25, 4.0);
        Self {
            style,
            motion,
            speed,
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

/// What a screen puts between itself and the world behind it (Cottleta's Settings).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackdropMode {
    /// As the game does it.
    #[default]
    Vanilla,
    /// Nothing: the world shows as it is.
    Off,
    /// Darkened, by `strength`.
    Fade,
    /// Blurred, by `strength`.
    Blur,
}

impl BackdropMode {
    pub const ALL: [BackdropMode; 4] = [
        BackdropMode::Vanilla,
        BackdropMode::Off,
        BackdropMode::Fade,
        BackdropMode::Blur,
    ];
    pub fn name(self) -> &'static str {
        match self {
            BackdropMode::Vanilla => "Vanilla",
            BackdropMode::Off => "Off",
            BackdropMode::Fade => "Fade",
            BackdropMode::Blur => "Blur",
        }
    }
}

/// A screen's backdrop: its mode and how strong (0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Backdrop {
    pub mode: BackdropMode,
    pub strength: f32,
}

impl Default for Backdrop {
    fn default() -> Self {
        Self {
            mode: BackdropMode::Vanilla,
            strength: 0.6,
        }
    }
}

impl Backdrop {
    pub fn to_text(self) -> String {
        format!("{}:{}", self.mode.name().to_lowercase(), self.strength)
    }
    pub fn parse(text: &str) -> Self {
        let (mode, strength) = text.split_once(':').unwrap_or((text, "0.6"));
        Self {
            mode: BackdropMode::ALL
                .into_iter()
                .find(|m| m.name().eq_ignore_ascii_case(mode))
                .unwrap_or_default(),
            strength: strength
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())
                .unwrap_or(0.6)
                .clamp(0.0, 1.0),
        }
    }
    /// The blur radius for `Blur` (1 to 32 GUI pixels by strength).
    pub fn blur_radius(self) -> f32 {
        1.0 + self.strength * 31.0
    }
}

thread_local! {
    /// The buttons' hover setting, set by the game each frame (`set_buttons`).
    static BUTTONS: std::cell::Cell<Setting> = std::cell::Cell::new(Setting {
        style: Style::Off,
        motion: Motion::Fade,
        speed: 1.0,
    });
    /// Each button's hover glow by its rectangle (whole GUI pixels), with when it was last
    /// drawn: buttons have no id that lasts, their place does while a screen is open.
    static GLOW: std::cell::RefCell<crate::fast_hash::FxHashMap<[i32; 4], (Tween, Instant)>> =
        std::cell::RefCell::default();
}

/// The setting [`hover`] eases buttons by, from now on.
pub fn set_buttons(setting: Setting) {
    BUTTONS.with(|b| b.set(setting));
}

/// How highlighted the button at `rect` is drawn (0 plain, 1 highlighted) while the pointer is
/// over it or not (`hovered`). Off: exactly `hovered`, as vanilla swaps the sprite.
pub fn hover(rect: [f32; 4], hovered: bool) -> f32 {
    let setting = BUTTONS.with(std::cell::Cell::get);
    let target = if hovered { 1.0 } else { 0.0 };
    if !setting.on() {
        return target;
    }
    let key = rect.map(|v| v.round() as i32);
    let now = Instant::now();
    GLOW.with_borrow_mut(|glow| {
        // Screens come and go: forget buttons not drawn for a second.
        if glow.len() > 256 {
            glow.retain(|_, (_, seen)| {
                now.saturating_duration_since(*seen) < Duration::from_secs(1)
            });
        }
        let (tween, seen) = glow.entry(key).or_insert_with(|| (Tween::at(target), now));
        *seen = now;
        tween.go(target, setting, now);
        tween.value(now).clamp(0.0, 1.0)
    })
}

/// Whether any button is still easing: a screen drawn with it can't be kept.
pub fn buttons_moving() -> bool {
    let now = Instant::now();
    GLOW.with_borrow(|glow| glow.values().any(|(tween, _)| tween.moving(now)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_style_starts_at_zero_and_ends_at_one_without_overshooting() {
        for style in Style::ALL {
            if style != Style::Off {
                assert!(style.ease(0.0).abs() < 1e-6, "{style:?}");
            }
            assert!((style.ease(1.0) - 1.0).abs() < 1e-6, "{style:?}");
            let peak = (0..=100)
                .map(|i| style.ease(i as f32 / 100.0))
                .fold(0.0, f32::max);
            assert!(peak <= 1.0 + 1e-6, "{style:?}");
        }
        assert!(Style::EaseIn.ease(0.25) < Style::Linear.ease(0.25));
        assert!(Style::EaseOut.ease(0.25) > Style::Linear.ease(0.25));
    }

    #[test]
    fn from_middle_grows_from_nothing_and_slides_come_from_their_side() {
        let size = [400.0, 200.0];
        assert_eq!(Motion::FromMiddle.pose(0.0, size), ([0.0, 0.0], 0.0, 0.0));
        assert_eq!(Motion::FromMiddle.pose(1.0, size), ([0.0, 0.0], 1.0, 1.0));
        assert_eq!(Motion::FromLeft.pose(0.0, size).0, [-200.0, 0.0]);
        assert_eq!(Motion::FromBottom.pose(0.0, size).0, [0.0, 100.0]);
        for motion in Motion::ALL {
            assert_eq!(motion.pose(1.0, size), ([0.0, 0.0], 1.0, 1.0), "{motion:?}");
        }
    }

    #[test]
    fn a_button_glows_in_and_out_and_off_is_vanilla() {
        set_buttons(Setting::default());
        assert_eq!(hover([0.0, 0.0, 200.0, 20.0], true), 1.0);
        set_buttons(Setting {
            style: Style::Linear,
            motion: Motion::Fade,
            speed: 0.25,
        });
        let rect = [10.0, 40.0, 200.0, 20.0];
        assert_eq!(hover(rect, false), 0.0);
        let first = hover(rect, true);
        assert!((0.0..0.5).contains(&first), "{first}");
        assert!(buttons_moving());
        set_buttons(Setting::default());
    }

    #[test]
    fn a_backdrop_saves_and_reads_back_and_defaults_to_vanilla() {
        let b = Backdrop {
            mode: BackdropMode::Blur,
            strength: 0.25,
        };
        assert_eq!(Backdrop::parse(&b.to_text()), b);
        assert_eq!(Backdrop::parse("junk").mode, BackdropMode::Vanilla);
        assert_eq!(Backdrop::parse("fade:7").strength, 1.0);
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
        let linear = Setting {
            style: Style::Linear,
            ..Setting::default()
        };
        let mut tween = Tween::at(0.0);
        tween.go(10.0, linear, start);
        let half = start + Style::Linear.duration(1.0) / 2;
        assert!((tween.value(half) - 5.0).abs() < 0.01);
        assert!(tween.moving(half));
        // Turned back halfway: from 5, not from 10.
        tween.go(0.0, linear, half);
        assert!((tween.value(half) - 5.0).abs() < 0.01);
        let end = half + Style::Linear.duration(1.0);
        assert_eq!(tween.value(end), 0.0);
        assert!(!tween.moving(end));
    }

    #[test]
    fn settings_save_and_read_back_and_bad_text_is_off() {
        let s = Setting {
            style: Style::EaseInOut,
            motion: Motion::FromMiddle,
            speed: 1.5,
        };
        assert_eq!(Setting::parse(&s.to_text()), s);
        assert_eq!(Setting::parse("nonsense"), Setting::default());
        assert_eq!(Setting::parse("easeout:fromleft:99").speed, 4.0);
        assert_eq!(Style::Linear.duration(2.0), Duration::from_millis(100));
    }
}
