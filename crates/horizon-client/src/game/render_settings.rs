//! The renderer and its settings: building it for the window, applying the video settings and a
//! shader pack's translucent layers, and the far terrain (LOD) distances.
use super::*;

#[derive(Default)]
pub(super) struct FrameAttempt<K = FrameKey>(Option<K>);

/// The frame the renderer would be made with (`Renderer::requested_frame`), the pick's path,
/// its outcome.
type FrameKey = (
    Option<horizon_client::gfx::FrameConfiguration>,
    PathBuf,
    Option<Option<horizon_shader_api::Api>>,
);

impl<K: PartialEq> FrameAttempt<K> {
    // Remember attempts, including failures, until the requested configuration changes.
    fn changed(&mut self, key: K) -> bool {
        let changed = self.0.as_ref() != Some(&key);
        self.0 = Some(key);
        changed
    }
}

impl Game<'_> {
    pub(super) fn renderer_for(
        &self,
        window: Arc<Window>,
        pack: &PackResources,
        gui: &UiResources,
    ) -> Result<Renderer> {
        build_renderer(
            window,
            self.options.offscreen,
            pack,
            gui,
            self.renderer.as_ref(),
            None,
            self.settings.horizon.api,
            self.settings.horizon.translucent_batching,
            &self.settings.horizon,
            shader_packs::picked_pack_shape(&self.settings),
            self.renderer
                .as_ref()
                .map(|r| r.display_reading())
                .unwrap_or_default(),
            (
                self.settings.file.int("textureFiltering"),
                self.settings.file.int("maxAnisotropyBit"),
            ),
        )
    }
    /// Under `HORIZON_HDR_FRAME=float` the frame follows what draws the picture
    /// (docs/design/hdr-linear-light.md, "When it changes"): when HDR, Smooth Gradients or the
    /// pick (unless refused) call for another frame than the renderer's, it is made again before
    /// this frame draws. Without the switch a frame stays until the next start, as before.
    pub(super) fn follow_frame(&mut self) {
        if !native_shaders::linear_frame_switch()
            || self.options.paced_by_split
            || self.options.split_guest
        {
            return;
        }
        let picked = shader_packs::picked_pack(&self.settings);
        let path = self.native_shaders.resolve_path(
            &Self::shader_packs_dir(),
            picked,
            shader_packs::dev_pack().is_some(),
        );
        let available = path.exists()
            && path.file_name().is_none_or(|f| {
                !shader_packs::active_pack(&f.to_string_lossy(), &self.settings.mods).is_empty()
            });
        let outcome = if available {
            native_shaders::outcome(&path)
        } else {
            Some(None)
        };
        // The renderer's offer can change while the game runs (Vulkan's formats follow desktop
        // HDR and the window's output), and the frame with it.
        let request = shader_packs::picked_pack_shape(&self.settings).frame();
        let wanted = self
            .renderer
            .as_ref()
            .map(|r| r.requested_frame(&self.settings.horizon, request));
        if !self.frame_attempted.changed((wanted, path, outcome)) {
            return;
        }
        if self
            .renderer
            .as_ref()
            .is_none_or(|r| r.frame_follows(&self.settings.horizon, request))
        {
            return;
        }
        if let Err(e) = self.rebuild_renderer() {
            log::warn!("Horizon: the renderer for the new frame could not be made: {e}");
        }
    }

    /// The renderer made again on the same device from the resources loaded, keeping the
    /// pipelines that come out the same (`adopt_renderer`, as after a lost device).
    fn rebuild_renderer(&mut self) -> Result<()> {
        let Some(window) = self.window.clone() else {
            return Ok(());
        };
        let started = std::time::Instant::now();
        if let Some(renderer) = &mut self.renderer {
            renderer.release_surface();
        }
        let mut renderer = match self.renderer_for(window, &self.pack, &self.gui) {
            Ok(renderer) => renderer,
            Err(error) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.recreate_surface()?;
                }
                return Err(error);
            }
        };
        if let Some(old) = &mut self.renderer {
            renderer.resize(old.config.width, old.config.height);
            renderer.screenshot = old.screenshot.take();
            renderer.screenshot_result = old.screenshot_result.take();
        }
        self.adopt_renderer(renderer);
        log::info!(
            "Horizon: the renderer was made again for the frame the settings call for in {} ms",
            started.elapsed().as_millis()
        );
        Ok(())
    }

    /// Measures this PC from the title screen (`auto_test`).
    pub(crate) fn start_auto_test(&mut self) {
        if self.title.is_none() || self.auto_testing {
            return;
        }
        self.auto_testing = true;
        self.auto_test_level = 0;
        self.auto_test_stopping = false;
        self.auto_test_control = self.control.is_none();
        if self.screen == Screen::Options {
            self.screen = Screen::Playing;
        }
        self.capture(false);
        horizon_client::auto_settings::save_for_test(&self.settings);
        let control = self
            .control
            .get_or_insert_with(horizon_client::control::Server::local);
        horizon_client::auto_test::start(control.sender(), std::env::current_dir().unwrap_or_default());
    }
    /// The test is over: the player's settings again, and back to the title.
    pub(crate) fn end_auto_test(&mut self) -> Result<()> {
        if !std::mem::take(&mut self.auto_testing) {
            return Ok(());
        }
        if self.title.is_none() {
            self.close_world()?;
        }
        horizon_client::auto_settings::restore_after_test(&mut self.settings);
        self.settings.save(&Self::options_path());
        self.apply_settings();
        Ok(())
    }
    /// Esc or the menu's Stop: the test sees the paused screen at its next check.
    pub(crate) fn stop_auto_test(&mut self) {
        self.auto_test_stopping = true;
        self.pause_menu = false;
        self.pause_focus = None;
        self.screen = Screen::Paused;
    }
    /// The test's menu over the world, the pointer free: what it's doing, and Stop.
    pub(crate) fn auto_test_stop_button(w: f32, h: f32) -> (f32, f32, f32) {
        ((w / 2.0).floor() - 60.0, (h / 2.0).floor() + 16.0, 120.0)
    }
    pub(crate) fn draw_auto_test(&mut self, batch: &mut HudBatch) {
        use horizon_client::{auto_settings, ui};
        let (w, h) = (batch.width, batch.height);
        let atlas = &self.gui.atlas;
        let (panel_w, panel_h) = (240.0, 84.0);
        let (x, y) = (((w - panel_w) / 2.0).floor(), (h / 2.0).floor() - 44.0);
        ui::nine_slice(batch, atlas, "gui/sprites/widget/text_field", x, y, panel_w, panel_h, u32::MAX);
        let center = |batch: &mut HudBatch, text: &str, y: f32| {
            let tw = HudBatch::text_width(atlas, text);
            batch.text(atlas, text, ((w - tw) / 2.0).floor(), y, u32::MAX, true);
        };
        center(batch, &horizon_client::lang::get_or("horizon.auto_test.title", "Measuring This PC"), y + 8.0);
        let picks = auto_settings::level_at(self.auto_test_level);
        let line = if self.auto_test_stopping {
            horizon_client::lang::get_or("horizon.auto_test.stopping", "Stopping...")
        } else {
            format!(
                "Level {} of {}: {}, {} chunks",
                self.auto_test_level + 1,
                auto_settings::LEVELS,
                picks.preset.name(),
                picks.render_distance,
            )
        };
        center(batch, &line, y + 22.0);
        let (bar_x, bar_y, bar_w) = (x + 20.0, y + 36.0, panel_w - 40.0);
        batch.fill(atlas, bar_x, bar_y, bar_w, 4.0, 0xFF40_4040);
        let done = (self.auto_test_level + 1) as f32 / auto_settings::LEVELS as f32;
        batch.fill(atlas, bar_x, bar_y, (bar_w * done).floor(), 4.0, 0xFF80_FF80);
        let (bx, by, bw) = Self::auto_test_stop_button(w, h);
        let hover = !self.auto_test_stopping && ui::hovered(self.cursor, bx, by, bw, 20.0);
        let label = horizon_client::text::Text::literal(horizon_client::lang::get_or(
            "horizon.auto_test.stop",
            "Stop",
        ));
        ui::button(batch, atlas, bx, by, bw, 20.0, &label, !self.auto_test_stopping, hover);
    }
    /// The Auto page, once the test's world has closed.
    pub(crate) fn show_auto_page(&mut self) {
        self.auto_test_drop_control = std::mem::take(&mut self.auto_test_control);
        if self.title.is_some() {
            self.open_options_at(true, horizon_client::options_ui::Page::Graphics);
            self.ui.options.show_graphics_page(horizon_client::graphics_ui::Page::Auto);
        }
    }
    pub(super) fn apply_settings(&mut self) {
        // Far terrain's tiles are meshed in the shape they have: start them over.
        let slopes = self.settings.horizon.far_shape.min_stride();
        if horizon_client::lod::slope_stride() != slopes {
            horizon_client::lod::set_slope_stride(slopes);
            self.clear_lod();
        }
        // LocalPlayer.refreshChatAbilities: the chat shows what the Chat option allows.
        let abilities = self.chat_abilities();
        self.ui.chat.set_abilities(&self.gui.atlas.font, abilities);
        let render_distance_changed = self.streaming.radius != self.settings.render_distance;
        self.streaming.radius = self.settings.render_distance;
        // Options.renderDistance, biomeBlendRadius, ambientOcclusion,
        // cutoutLeaves, Opaque Ice and the leaf-culling mode:
        // LevelExtractor.allChanged when they change.
        let leaf_culling = self.effective_leaf_culling();
        let (s, st) = (&self.settings, &self.streaming);
        let reload = render_distance_changed
            || (
                s.biome_blend,
                s.smooth_lighting,
                s.cutout_leaves,
                s.horizon.opaque_ice,
                leaf_culling,
            ) != (
                st.biome_blend,
                st.smooth_lighting,
                st.cutout_leaves,
                st.opaque_ice,
                st.leaf_culling,
            );
        self.streaming.biome_blend = self.settings.biome_blend;
        self.streaming.smooth_lighting = self.settings.smooth_lighting;
        self.streaming.cutout_leaves = self.settings.cutout_leaves;
        self.streaming.opaque_ice = self.settings.horizon.opaque_ice;
        self.streaming.leaf_culling = leaf_culling;
        if reload {
            self.all_sections_changed();
        }
        self.audio.master = self.settings.master_volume;
        if self.settings.sound_device != self.audio.device() {
            let _ = self.audio.set_device(&self.settings.sound_device);
        }
        horizon_client::hud::GUI_SCALE.store(
            self.settings.gui_scale,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.apply_ui_settings();
    }
    /// The sprite layers a shader pack draws in the translucent pass: every section is meshed
    /// again when they change.
    pub(super) fn set_translucent_layers(&mut self, layers: Vec<bool>) {
        if *layers != *self.streaming.translucent_layers {
            self.streaming.translucent_layers = layers.into();
            self.all_sections_changed();
        }
    }

    /// LOD Distance and Far Detail Reduced as the player chose them, the
    /// distance no farther than a server sampling far terrain for us allows.
    pub(super) fn far_terrain_choice(&self) -> (u32, bool) {
        let g = &self.settings.horizon;
        let reduced = g.far_detail == horizon_client::graphics::FarDetail::Reduced;
        let distance = match self.server_far_terrain() {
            Some(offer) => g.lod_distance.min(offer.distance),
            None => g.lod_distance,
        };
        (distance, reduced)
    }
    /// The far terrain another machine's server samples for us
    /// (`lod_net`): a client of a dedicated server or of a LAN world, not of
    /// its own server thread, which hands its generator over directly.
    pub(super) fn server_far_terrain(&self) -> Option<horizon_client::lod_net::Offer> {
        if self.integrated.is_some() {
            return None;
        }
        self.remote.as_ref()?.far_terrain
    }
    /// LOD Distance as far terrain uses it (`graphics::lod_distance_capped`).
    pub(super) fn lod_distance(&self) -> u32 {
        let (distance, _) = self.far_terrain_choice();
        match &self.renderer {
            Some(renderer) => {
                horizon_client::graphics::lod_distance_capped(distance, &renderer.info)
            }
            None => distance.min(horizon_client::lod::MAX_CHUNKS),
        }
    }
    /// Only enabled LOD worlds extend the renderer's range, never simulation.
    pub(super) fn terrain_distance(&self) -> i32 {
        if self.lod.is_some() {
            self.streaming.radius.max(self.lod_distance() as i32)
        } else {
            self.streaming.radius
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FrameAttempt;
    use horizon_shader_api::Api;

    #[test]
    fn failed_frame_attempt_waits_for_a_policy_or_setting_change() {
        let mut attempts = FrameAttempt::default();
        let mut rebuilds = 0;
        // The requested frame stands for itself here: the 8-bit one, then the float one.
        for key in [
            ("8-bit", "p", None),
            ("8-bit", "p", None),
            // HDR output offered later, with HDR On: the frame is looked at again.
            ("float", "p", None),
            ("float", "p", Some(Some(Api::V0_2))),
            ("float", "p", Some(Some(Api::V0_2))),
            ("float", "p", Some(None)),
            ("float", "p", Some(None)),
            ("float", "q", None),
            ("8-bit", "q", None),
        ] {
            if attempts.changed(key) {
                // A replacement fails: no success is recorded, just the attempted key.
                rebuilds += 1;
            }
        }
        assert_eq!(rebuilds, 6);
    }
}
