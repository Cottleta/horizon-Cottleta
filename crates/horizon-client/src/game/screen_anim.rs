//! Cottleta's Settings: the inventory and containers open and close animated (`gui_anim`).
//! Off (the default) nothing here changes a vertex: the screen appears and goes at once.
use super::{Game, Screen};
use horizon_client::gui_anim::{Setting, Tween};
use horizon_client::hud::{Blend, HudBatch, HudVertex};
use horizon_client::screen_video;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

/// The player's pictures: the renderer's name for each, and its file's name in `pictures/`.
const PICTURES: [(&str, &str); 4] = [
    ("loading", "loading"),
    ("world", "world_loading"),
    ("inventory", "inventory"),
    ("menu", "menu"),
];

/// Where the player puts their pictures: `pictures/` in the game folder.
pub(super) fn pictures_dir() -> PathBuf {
    horizon_client::logging::game_home().join("pictures")
}

/// The picture or video file named `stem` in `dir`, in any format the game reads.
fn picture_file(dir: &std::path::Path, stem: &str) -> Option<PathBuf> {
    ["png", "jpg", "jpeg", "webp"]
        .into_iter()
        .chain(screen_video::FFMPEG_EXTENSIONS)
        .map(|ext| dir.join(format!("{stem}.{ext}")))
        .find(|p| p.is_file())
}

/// A picture as it arrives from its file.
enum Arrival {
    Still([u32; 2], Vec<u8>),
    Video(mpsc::Receiver<screen_video::Frame>),
    /// No file, or its setting is off.
    Gone,
}

/// The file `stem` in `dir` opened: a still decoded, a video started.
fn open_picture(dir: &std::path::Path, stem: &str) -> Arrival {
    let Some(path) = picture_file(dir, stem) else {
        return Arrival::Gone;
    };
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if screen_video::FFMPEG_EXTENSIONS.contains(&ext.as_str()) {
        return screen_video::play_ffmpeg(&path, dir).map_or(Arrival::Gone, Arrival::Video);
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return Arrival::Gone;
    };
    if ext == "webp" && screen_video::is_animated_webp(&bytes) {
        return screen_video::play_webp(bytes).map_or(Arrival::Gone, Arrival::Video);
    }
    match horizon_client::hud::decode_image_bytes(&bytes) {
        Ok((w, h, rgba)) => Arrival::Still([w, h], rgba),
        Err(_) => Arrival::Gone,
    }
}

/// A video playing: its frames, and when the next one is due.
struct Playing {
    frames: mpsc::Receiver<screen_video::Frame>,
    next: Option<Instant>,
}

/// Loading the player's pictures off the frame's thread.
#[derive(Default)]
pub(super) struct Pictures {
    /// Which the settings wanted when they were last loaded.
    wanted: Option<[bool; 4]>,
    /// Opened pictures by name.
    arriving: Option<mpsc::Receiver<(&'static str, Arrival)>>,
    /// The videos playing, by name.
    playing: Vec<(&'static str, Playing)>,
}

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
    /// How far open it is (0 to 1).
    pub(super) fn progress(&self, now: Instant) -> f32 {
        self.tween.value(now).clamp(0.0, 1.0)
    }
}

/// A loading frame's picture (`loading` or `world`), with the screen's own background taken
/// out of `hud`; None while the setting is off or the picture isn't loaded.
pub(super) fn loading_picture(
    on: bool,
    name: &str,
    renderer: &horizon_client::gfx::Renderer,
    hud: &mut HudBatch,
) -> Option<horizon_client::screen_picture::Draw> {
    if !on || !renderer.has_picture(name) {
        return None;
    }
    clear_backdrops(hud, 0);
    Some(horizon_client::screen_picture::Draw {
        name: name.to_owned(),
        alpha: 1.0,
        at: None,
        rect: None,
    })
}

/// The quads from `from` drawn from the sprites `wanted` names (a container's own texture,
/// `gui/container/...`, the panel with its grid of slots) made clear: the first one's vertex
/// and their box in GUI pixels.
fn take_sprites(
    hud: &mut HudBatch,
    atlas: &horizon_client::hud::HudAtlas,
    from: usize,
    wanted: impl Fn(&str) -> bool,
) -> Option<(usize, [f32; 4])> {
    let panels: Vec<[f32; 4]> = atlas
        .sprites
        .iter()
        .filter(|(name, _)| wanted(name))
        .map(|(_, r)| [r.u0, r.v0, r.u1, r.v1])
        .collect();
    let from = from.min(hud.vertices.len());
    let (mut first, mut lo, mut hi) = (None, [f32::MAX; 2], [f32::MIN; 2]);
    for (i, quad) in hud.vertices[from..].chunks_mut(6).enumerate() {
        let n = quad.len() as f32;
        let uv = quad
            .iter()
            .fold([0.0f32; 2], |a, v| [a[0] + v.uv[0] / n, a[1] + v.uv[1] / n]);
        let panel = panels
            .iter()
            .any(|r| uv[0] >= r[0] && uv[0] <= r[2] && uv[1] >= r[1] && uv[1] <= r[3]);
        if !panel {
            continue;
        }
        first.get_or_insert(from + i * 6);
        for v in quad {
            for a in 0..2 {
                lo[a] = lo[a].min(v.pos[a]);
                hi[a] = hi[a].max(v.pos[a]);
            }
            v.color &= 0x00FF_FFFF;
        }
    }
    Some((first?, [lo[0], lo[1], hi[0] - lo[0], hi[1] - lo[1]]))
}

/// The quads from `from` that cover the whole screen (a screen's darkened or filled
/// background) made clear: the player's picture shows there instead.
fn clear_backdrops(hud: &mut HudBatch, from: usize) {
    let size = [hud.width, hud.height];
    let from = from.min(hud.vertices.len());
    for quad in hud.vertices[from..].chunks_mut(6) {
        if quad.len() == 6 && covers(quad, size) {
            for v in quad {
                v.color &= 0x00FF_FFFF;
            }
        }
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
    /// The backdrop Cottleta's Settings give the open screen: the containers' or the pause
    /// menu's; None for any other screen.
    pub(super) fn screen_backdrop(&self) -> Option<horizon_client::gui_anim::Backdrop> {
        let g = &self.settings.horizon;
        match self.screen {
            Screen::Inventory | Screen::Creative => Some(g.backdrop_containers),
            Screen::Paused => Some(g.backdrop_pause),
            _ => None,
        }
    }
    /// Before `screen` draws: the pause menu's own darkening when it is set to fade (vanilla's
    /// has none of its own).
    pub(super) fn backdrop_before(&self, batch: &mut HudBatch, screen: Screen) {
        use horizon_client::gui_anim::BackdropMode;
        let b = self.settings.horizon.backdrop_pause;
        if screen == Screen::Paused && b.mode == BackdropMode::Fade {
            let alpha = (b.strength * 255.0).round() as u8;
            let (w, h) = (batch.width, batch.height);
            batch.fill(
                &self.gui.atlas,
                0.0,
                0.0,
                w,
                h,
                horizon_client::hud::rgba(0, 0, 0, alpha),
            );
        }
    }
    /// After `screen` drew from `start`: the containers' darkened backdrop as their setting
    /// says (gone when off or blurred, by its strength when faded).
    pub(super) fn backdrop_after(&self, batch: &mut HudBatch, start: usize, screen: Screen) {
        use horizon_client::gui_anim::BackdropMode;
        if !matches!(screen, Screen::Inventory | Screen::Creative) {
            return;
        }
        let b = self.settings.horizon.backdrop_containers;
        let alpha = match b.mode {
            BackdropMode::Vanilla => return,
            BackdropMode::Off | BackdropMode::Blur => 0,
            BackdropMode::Fade => (b.strength * 255.0).round() as u32,
        };
        let size = [batch.width, batch.height];
        let start = start.min(batch.vertices.len());
        for quad in batch.vertices[start..].chunks_mut(6) {
            if quad.len() == 6 && covers(quad, size) {
                for v in quad {
                    v.color = (v.color & 0x00FF_FFFF) | alpha.min(255) << 24;
                }
            }
        }
    }
    /// Loads the pictures the settings ask for when that changes (or `again`), on a thread,
    /// and hands the renderer those that arrived.
    pub(super) fn poll_pictures(&mut self, again: bool) {
        let g = &self.settings.horizon;
        let wanted = [
            g.picture_loading,
            g.picture_world,
            g.picture_inventory,
            g.picture_menu,
        ];
        if again || self.pictures.wanted != Some(wanted) {
            self.pictures.wanted = Some(wanted);
            // Dropping a video's frames ends its thread (and FFmpeg).
            self.pictures.playing.clear();
            let (tx, rx) = mpsc::channel();
            self.pictures.arriving = Some(rx);
            let jobs: Vec<_> = PICTURES
                .iter()
                .zip(wanted)
                .map(|(&(n, f), on)| (n, f, on))
                .collect();
            std::thread::Builder::new()
                .name("Pictures".into())
                .spawn(move || {
                    let dir = pictures_dir();
                    for (name, file, on) in jobs {
                        let arrival = if on {
                            open_picture(&dir, file)
                        } else {
                            Arrival::Gone
                        };
                        if tx.send((name, arrival)).is_err() {
                            return;
                        }
                    }
                })
                .ok();
        }
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        if let Some(rx) = &self.pictures.arriving {
            while let Ok((name, arrival)) = rx.try_recv() {
                self.pictures.playing.retain(|(n, _)| *n != name);
                match arrival {
                    Arrival::Still(size, rgba) => renderer.upload_picture(name, size, &rgba),
                    Arrival::Video(frames) => {
                        renderer.remove_picture(name);
                        self.pictures
                            .playing
                            .push((name, Playing { frames, next: None }));
                    }
                    Arrival::Gone => renderer.remove_picture(name),
                }
            }
        }
        // A video's next frame when it's due, while it shows (and its first one at once).
        let now = Instant::now();
        self.pictures.playing.retain_mut(|(name, video)| {
            let showing = !renderer.has_picture(name)
                || renderer.picture_drawn(name).is_some_and(|at| {
                    now.duration_since(at) < std::time::Duration::from_millis(250)
                });
            if !showing {
                // Starts again where it stopped, not racing to catch up.
                video.next = None;
                return true;
            }
            if video.next.is_some_and(|due| now < due) {
                return true;
            }
            match video.frames.try_recv() {
                Ok(frame) => {
                    renderer.upload_picture(name, frame.size, &frame.rgba);
                    let due = video.next.unwrap_or(now) + frame.shows_for;
                    video.next = Some(if due < now { now } else { due });
                    true
                }
                Err(mpsc::TryRecvError::Empty) => true,
                Err(mpsc::TryRecvError::Disconnected) => false,
            }
        });
    }
    /// This frame's pictures, with what each replaces taken out of `hud`: the world's while
    /// one loads; else the main menu's background; and the inventory's panel while
    /// it (or its closing) shows.
    pub(super) fn frame_pictures(
        &self,
        hud: &mut HudBatch,
    ) -> Vec<horizon_client::screen_picture::Draw> {
        use horizon_client::screen_picture::Draw;
        let mut draws = Vec::new();
        let Some(renderer) = self.renderer.as_ref() else {
            return draws;
        };
        let g = &self.settings.horizon;
        if !(g.picture_world || g.picture_inventory || g.picture_menu) {
            return draws;
        }
        let now = Instant::now();
        let s = hud.scale;
        // Joining a world (a server's, or single player's own) until the player is in it, and
        // while its terrain loads after a dimension change.
        let joining = self
            .remote
            .as_ref()
            .is_some_and(|r| self.player.is_none() || r.level_load.is_active());
        if g.picture_world && joining && renderer.has_picture("world") {
            clear_backdrops(hud, 0);
            draws.push(Draw {
                name: "world".to_owned(),
                alpha: 1.0,
                at: None,
                rect: None,
            });
            return draws;
        }
        if self.title.is_some() {
            // Over the panorama, under everything the menu draws.
            if g.picture_menu && renderer.has_picture("menu") {
                draws.push(Draw {
                    name: "menu".to_owned(),
                    alpha: 1.0,
                    at: Some(0),
                    rect: None,
                });
            }
            return draws;
        }
        let inventory = matches!(self.screen, Screen::Inventory | Screen::Creative);
        let closing = !inventory && g.anim_containers.on() && self.screen_anim.moving(now);
        if !(g.picture_inventory && (inventory || closing) && renderer.has_picture("inventory")) {
            return draws;
        }
        // In place of the panel's own texture (`gui/container/...`): the dark backdrop around
        // it, the slots' highlights and the items stay.
        let from = hud.blur_at as usize;
        let Some((first, rect)) = take_sprites(hud, &self.gui.atlas, from, |name| {
            name.starts_with("gui/container/")
        }) else {
            return draws;
        };
        let alpha = if g.anim_containers.on() {
            self.screen_anim.progress(now)
        } else {
            1.0
        };
        draws.push(Draw {
            name: "inventory".to_owned(),
            alpha,
            at: Some(first as u32),
            rect: Some([rect[0] * s, rect[1] * s, rect[2] * s, rect[3] * s]),
        });
        draws
    }
    /// Which menu page is open: the screen, the options page and the title screen's mode.
    fn menu_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::mem::discriminant(&self.screen).hash(&mut h);
        if self.screen == Screen::Options {
            std::mem::discriminant(&self.ui.options.page()).hash(&mut h);
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
        pose(
            &mut batch.vertices[start..],
            setting,
            self.menu_anim.tween.value(now),
            size,
        );
    }
    /// The hotbar's selection box in `draws` moved to where it is gliding (`anim_hotbar`).
    pub(super) fn slide_hotbar_selection(
        &mut self,
        draws: &mut [horizon_client::hud_layout::GuiDraw],
    ) {
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
    pub(super) fn screen_anim_begin(
        &mut self,
        batch: &mut HudBatch,
        screen: Screen,
    ) -> (usize, usize) {
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
    pub(super) fn screen_anim_end(
        &mut self,
        batch: &mut HudBatch,
        (start, blends_start): (usize, usize),
    ) {
        let setting = self.settings.horizon.anim_containers;
        if !setting.on() || !self.screen_anim.open {
            return;
        }
        let anim = &mut self.screen_anim;
        anim.last.clear();
        anim.last.extend_from_slice(&batch.vertices[start..]);
        anim.last_blends.clear();
        let first = start as u32;
        anim.last_blends
            .extend(batch.blends[blends_start..].iter().map(|(r, b)| {
                (
                    r.start.saturating_sub(first)..r.end.saturating_sub(first),
                    *b,
                )
            }));
        let now = Instant::now();
        if anim.tween.moving(now) {
            let size = [batch.width, batch.height];
            pose(
                &mut batch.vertices[start..],
                setting,
                anim.tween.value(now),
                size,
            );
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
            assert!(
                v.iter()
                    .zip(&before)
                    .all(|(a, b)| a.pos == b.pos && a.color == b.color)
            );
        }
    }
}
