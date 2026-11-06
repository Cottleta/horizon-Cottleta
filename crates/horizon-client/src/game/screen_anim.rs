//! Cottleta's Settings: the inventory and containers open and close animated (`gui_anim`).
//! Off (the default) nothing here changes a vertex: the screen appears and goes at once.
use super::{Game, Screen};
use horizon_client::gui_anim::{Setting, Tween};
use horizon_client::hud::{Blend, HudBatch, HudVertex};
use std::time::Instant;

/// The animated screen's state between frames.
pub(super) struct ScreenAnim {
    /// 0 closed, 1 open.
    tween: Tween,
    /// Whether the inventory or a container was open on the last frame.
    open: bool,
    /// The last open frame's vertices and blend runs (from 0), shown shrinking as it closes.
    last: Vec<HudVertex>,
    last_blends: Vec<(std::ops::Range<u32>, Blend)>,
}

/// The menu page's easing in (`anim_menus`): which page it is, and its progress.
pub(super) struct MenuAnim {
    key: u64,
    tween: Tween,
}

impl Default for MenuAnim {
    fn default() -> Self {
        Self {
            key: 0,
            tween: Tween::at(1.0),
        }
    }
}

impl MenuAnim {
    pub(super) fn moving(&self, now: Instant) -> bool {
        self.tween.moving(now)
    }
}

impl Default for ScreenAnim {
    fn default() -> Self {
        Self {
            tween: Tween::at(0.0),
            open: false,
            last: Vec::new(),
            last_blends: Vec::new(),
        }
    }
}

impl ScreenAnim {
    /// Whether a frame still moves it: the HUD can't be kept from the last frame then.
    pub(super) fn moving(&self, now: Instant) -> bool {
        self.tween.moving(now)
    }
}

/// Whether the vertices of `quad` cover the whole `size`: the screen's darkened background,
/// which only fades.
fn covers(quad: &[HudVertex], size: [f32; 2]) -> bool {
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for v in quad {
        for a in 0..2 {
            lo[a] = lo[a].min(v.pos[a]);
            hi[a] = hi[a].max(v.pos[a]);
        }
    }
    lo[0] <= 0.0 && lo[1] <= 0.0 && hi[0] >= size[0] && hi[1] >= size[1]
}

/// `vertices` at progress `p` of `setting`'s motion on a screen of `size`: moved and scaled
/// about the middle, faded; whole-screen quads only faded.
fn pose(vertices: &mut [HudVertex], setting: Setting, p: f32, size: [f32; 2]) {
    let (offset, scale, alpha) = setting.motion.pose(p, size);
    let middle = [size[0] / 2.0, size[1] / 2.0];
    let fade = |color: u32| {
        let a = ((color >> 24) as f32 * alpha.clamp(0.0, 1.0)).round() as u32;
        (color & 0x00FF_FFFF) | a.min(255) << 24
    };
    for quad in vertices.chunks_mut(6) {
        let background = quad.len() == 6 && covers(quad, size);
        for v in quad {
            if !background {
                for a in 0..2 {
                    v.pos[a] = middle[a] + (v.pos[a] - middle[a]) * scale + offset[a];
                }
            }
            v.color = fade(v.color);
        }
    }
}

impl Game<'_> {
    /// Which menu page is open: the screen, the options page and the title screen's mode.
    fn menu_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::mem::discriminant(&self.screen).hash(&mut h);
        if self.screen == Screen::Options {
            std::mem::discriminant(&self.ui.options.page()).hash(&mut h);
            std::mem::discriminant(&self.ui.options.graphics.shown_page()).hash(&mut h);
        }
        if let Some(title) = &self.title {
            std::mem::discriminant(&title.mode).hash(&mut h);
        }
        h.finish()
    }
    /// Before a menu page draws: notes a new page. Where its vertices begin.
    pub(super) fn menu_anim_begin(&mut self, batch: &HudBatch) -> usize {
        let setting = self.settings.horizon.anim_menus;
        if setting.on() {
            let key = self.menu_key();
            if key != self.menu_anim.key {
                self.menu_anim.key = key;
                self.menu_anim.tween = Tween::at(0.0);
                self.menu_anim.tween.go(1.0, setting, Instant::now());
            }
        }
        batch.vertices.len()
    }
    /// After the page drew from `start`: poses it while it eases in.
    pub(super) fn menu_anim_end(&mut self, batch: &mut HudBatch, start: usize) {
        let setting = self.settings.horizon.anim_menus;
        let now = Instant::now();
        if !setting.on() || !self.menu_anim.tween.moving(now) {
            return;
        }
        let size = [batch.width, batch.height];
        pose(&mut batch.vertices[start..], setting, self.menu_anim.tween.value(now), size);
    }
    /// The hotbar's selection box in `draws` moved to where it is gliding (`anim_hotbar`).
    pub(super) fn slide_hotbar_selection(&mut self, draws: &mut [horizon_client::hud_layout::GuiDraw]) {
        let setting = self.settings.horizon.anim_hotbar;
        let now = Instant::now();
        let target = self.selected as f32;
        if self.hotbar_slide.target() != target {
            self.hotbar_slide.go(target, setting, now);
        }
        if !setting.on() {
            return;
        }
        let shift = ((self.hotbar_slide.value(now) - target) * 20.0).round() as i32;
        if shift == 0 {
            return;
        }
        for d in draws {
            if let horizon_client::hud_layout::GuiDraw::Sprite { sprite, x, .. } = d
                && sprite == "hud/hotbar_selection"
            {
                *x += shift;
            }
        }
    }
    /// Before the open screen draws: notes an opening or closing, and draws a closing one.
    /// The vertex and blend counts it returns are where the open screen's own begin.
    pub(super) fn screen_anim_begin(&mut self, batch: &mut HudBatch, screen: Screen) -> (usize, usize) {
        let setting = self.settings.horizon.anim_containers;
        let now = Instant::now();
        let open = matches!(screen, Screen::Inventory | Screen::Creative);
        let anim = &mut self.screen_anim;
        if open != anim.open {
            anim.open = open;
            if open {
                anim.tween = Tween::at(0.0);
                anim.tween.go(1.0, setting, now);
            } else {
                anim.tween.go(0.0, setting, now);
            }
        }
        if !open && setting.on() && anim.tween.moving(now) && !anim.last.is_empty() {
            let size = [batch.width, batch.height];
            let first = batch.vertices.len() as u32;
            let mut closing = anim.last.clone();
            pose(&mut closing, setting, anim.tween.value(now), size);
            batch.vertices.extend_from_slice(&closing);
            for (range, blend) in &anim.last_blends {
                batch
                    .blends
                    .push((range.start + first..range.end + first, *blend));
            }
        }
        (batch.vertices.len(), batch.blends.len())
    }
    /// After the open screen drew from `start` (vertices) and `blends_start`: poses it, and
    /// keeps a copy for its closing.
    pub(super) fn screen_anim_end(&mut self, batch: &mut HudBatch, (start, blends_start): (usize, usize)) {
        let setting = self.settings.horizon.anim_containers;
        if !setting.on() || !self.screen_anim.open {
            return;
        }
        let anim = &mut self.screen_anim;
        anim.last.clear();
        anim.last.extend_from_slice(&batch.vertices[start..]);
        anim.last_blends.clear();
        let first = start as u32;
        anim.last_blends.extend(
            batch.blends[blends_start..]
                .iter()
                .map(|(r, b)| (r.start.saturating_sub(first)..r.end.saturating_sub(first), *b)),
        );
        let now = Instant::now();
        if anim.tween.moving(now) {
            let size = [batch.width, batch.height];
            pose(&mut batch.vertices[start..], setting, anim.tween.value(now), size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::gui_anim::{Motion, Style};

    fn quad(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<HudVertex> {
        [[x0, y0], [x1, y0], [x1, y1], [x0, y0], [x1, y1], [x0, y1]]
            .into_iter()
            .map(|pos| HudVertex {
                pos,
                uv: [0.0; 2],
                color: 0xFF00_0000,
                depth: 0.0,
                glint_uv: [0.0; 2],
                text: 0,
            })
            .collect()
    }

    #[test]
    fn from_middle_shrinks_the_panel_to_the_middle_and_only_fades_the_background() {
        let size = [200.0, 100.0];
        let mut v = quad(0.0, 0.0, 200.0, 100.0);
        v.extend(quad(50.0, 25.0, 150.0, 75.0));
        let setting = Setting {
            style: Style::Linear,
            motion: Motion::FromMiddle,
            speed: 1.0,
        };
        pose(&mut v, setting, 0.5, size);
        // The background keeps its place, half faded.
        assert_eq!(v[2].pos, [200.0, 100.0]);
        assert_eq!(v[0].color >> 24, 128);
        // The panel at half size about the middle (100, 50).
        assert_eq!(v[6].pos, [75.0, 37.5]);
        assert_eq!(v[8].pos, [125.0, 62.5]);
    }

    #[test]
    fn in_place_nothing_moves() {
        let size = [200.0, 100.0];
        let before = quad(50.0, 25.0, 150.0, 75.0);
        for motion in Motion::ALL {
            let mut v = before.clone();
            let setting = Setting {
                style: Style::EaseOut,
                motion,
                speed: 1.0,
            };
            pose(&mut v, setting, 1.0, size);
            assert!(v.iter().zip(&before).all(|(a, b)| a.pos == b.pos && a.color == b.color));
        }
    }
}
