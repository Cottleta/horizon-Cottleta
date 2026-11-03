//! Auto settings: the most frames this PC can reach, or the best graphics that should
//! still reach a frame rate the player picks, and the player's own saved settings.
//!
//! Nothing is tested here. What this PC can reach comes from the frame times of play so
//! far, scaled by what each level of `LADDER` costs; before any play, from its parts.
use crate::graphics::Preset;
use crate::settings::Settings;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gpu {
    Discrete,
    Integrated,
    /// A CPU drawing: WARP, llvmpipe.
    Software,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hardware {
    pub gpu: Gpu,
    pub threads: usize,
    pub cores: usize,
    pub memory_gib: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picks {
    pub preset: Preset,
    pub render_distance: i32,
    pub simulation_distance: i32,
    /// The share of the first level's frame rate this one keeps.
    pub cost: f32,
}

const fn level(preset: Preset, render_distance: i32, simulation_distance: i32, cost: f32) -> Picks {
    Picks {
        preset,
        render_distance,
        simulation_distance,
        cost,
    }
}

/// The levels from most frames to best graphics. The costs are measured over terrain in
/// single player on one fast PC (7800X3D, RX 9070): a slower GPU will likely lose more.
const LADDER: [Picks; 5] = [
    level(Preset::Performance, 8, 6, 1.0),
    level(Preset::Performance, 12, 6, 0.89),
    level(Preset::Balanced, 12, 8, 0.85),
    level(Preset::Quality, 16, 12, 0.72),
    level(Preset::Ultra, 24, 12, 0.49),
];

/// The most frames this PC can reach.
pub fn best_performance() -> Picks {
    LADDER[0]
}

/// The best level expected to reach `target` at the first level's `capacity` frames a second.
fn level_for(target: u32, capacity: f32) -> Picks {
    LADDER
        .into_iter()
        .rev()
        .find(|level| capacity * level.cost >= target as f32)
        .unwrap_or(LADDER[0])
}

/// The best level expected to reach `target` on this PC.
pub fn for_target(target: u32) -> Picks {
    level_for(target, capacity())
}

/// What the first level should reach, from a PC's parts alone.
fn guess(hw: Hardware) -> f32 {
    match hw.gpu {
        Gpu::Software => 60.0,
        Gpu::Integrated => 300.0,
        Gpu::Unknown => 500.0,
        Gpu::Discrete if hw.cores < 6 => 1000.0,
        Gpu::Discrete => 2000.0,
    }
}

/// The frame rate at the first level, in frames a second as an `f32`'s bits; 0 before play.
static MEASURED: AtomicU32 = AtomicU32::new(0);

/// A frame of play: how long it took, and the CPU's and the GPU's milliseconds. A frame
/// that wasn't capped took what the PC needs. A capped one only shows its two busiest
/// parts, and what submitting and presenting add to them.
pub fn note_frame(frame_ms: f32, cpu_ms: f32, gpu_ms: f32, s: &Settings) {
    let capped = s.vsync || s.max_fps < 260;
    let work = if capped {
        cpu_ms.max(gpu_ms) * 1.5
    } else {
        frame_ms
    };
    if work <= 0.0 {
        return;
    }
    let now = 1000.0 / work / cost_of(s);
    let was = f32::from_bits(MEASURED.load(Ordering::Relaxed));
    let next = if was == 0.0 {
        now
    } else {
        was + (now - was) * 0.01
    };
    MEASURED.store(next.to_bits(), Ordering::Relaxed);
}

/// The cost of the level nearest these settings.
fn cost_of(s: &Settings) -> f32 {
    LADDER
        .into_iter()
        .filter(|level| s.horizon.preset == Preset::Custom || level.preset == s.horizon.preset)
        .min_by_key(|level| (level.render_distance - s.render_distance).abs())
        .map_or(1.0, |level| level.cost)
}

/// What the first level should reach here.
pub fn capacity() -> f32 {
    match f32::from_bits(MEASURED.load(Ordering::Relaxed)) {
        0.0 => guess(detect()),
        measured => measured,
    }
}

/// The lowest target anyone picks.
pub const MIN_TARGET: u32 = 30;

/// This PC's highest target.
fn max_target() -> u32 {
    round_target(capacity().max(60.0) as u32)
}

/// Fives up to 300 frames a second and tens past them: a slider's pixels reach no finer.
fn round_target(fps: u32) -> u32 {
    let step = if fps < 300 { 5 } else { 10 };
    (fps + step / 2) / step * step
}

/// The target within this PC's range.
pub fn clamp_target(fps: u32) -> u32 {
    fps.clamp(MIN_TARGET, max_target().max(MIN_TARGET))
}

/// The target set, or a middle one before any was.
pub fn target(s: &Settings) -> u32 {
    match s.horizon.auto_target_fps {
        0 => target_at(0.5),
        fps => clamp_target(fps),
    }
}

/// The target at a slider position, even steps of it multiplying the frame rate alike.
pub fn target_at(position: f32) -> u32 {
    target_between(position, MIN_TARGET, max_target())
}

fn target_between(position: f32, low: u32, high: u32) -> u32 {
    let (low_f, high_f) = (low as f32, high.max(low + 1) as f32);
    round_target((low_f * (high_f / low_f).powf(position.clamp(0.0, 1.0))).round() as u32)
        .clamp(low, high.max(low))
}

/// The slider position of a target; 0 picks the middle.
pub fn target_position(fps: u32) -> f32 {
    position_between(fps, MIN_TARGET, max_target())
}

fn position_between(fps: u32, low: u32, high: u32) -> f32 {
    if fps == 0 {
        return 0.5;
    }
    let (low_f, high_f) = (low as f32, high.max(low + 1) as f32);
    ((fps.clamp(low, high.max(low)) as f32 / low_f).ln() / (high_f / low_f).ln()).clamp(0.0, 1.0)
}

static GPU: AtomicU8 = AtomicU8::new(3);

/// The renderer's adapter, for `detect`.
pub fn note_gpu(info: &wgpu::AdapterInfo) {
    let gpu = match info.device_type {
        wgpu::DeviceType::DiscreteGpu => Gpu::Discrete,
        wgpu::DeviceType::IntegratedGpu => Gpu::Integrated,
        wgpu::DeviceType::Cpu => Gpu::Software,
        _ => Gpu::Unknown,
    };
    GPU.store(gpu as u8, Ordering::Relaxed);
}

/// This PC, as far as the system says. What it doesn't say counts as the middle of the road.
pub fn detect() -> Hardware {
    let gpu = match GPU.load(Ordering::Relaxed) {
        0 => Gpu::Discrete,
        1 => Gpu::Integrated,
        2 => Gpu::Software,
        _ => Gpu::Unknown,
    };
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    Hardware {
        gpu,
        threads,
        cores: horizon_threads::physical_cores().unwrap_or(threads),
        memory_gib: horizon_threads::total_memory().map_or(16, |bytes| (bytes + (1 << 29)) >> 30),
    }
}

/// Sets a level; a PC with little memory stays at a short render distance.
pub fn apply(picks: Picks, s: &mut Settings) {
    crate::graphics::apply_preset(picks.preset, s);
    s.horizon.preset = picks.preset;
    let most = if detect().memory_gib < 8 { 8 } else { i32::MAX };
    s.render_distance = picks.render_distance.min(most);
    s.simulation_distance = picks.simulation_distance;
    s.horizon.dynamic_hud = true;
}

fn saved_options() -> PathBuf {
    crate::logging::game_home()
        .join("saved-settings")
        .join("options.txt")
}

pub fn saved_exists() -> bool {
    saved_options().exists()
}

/// Keeps the settings the graphics screen edits, for `load`.
pub fn save(s: &Settings) {
    let path = saved_options();
    let Some(dir) = path.parent() else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&path, s.serialize()))
    {
        log::warn!("Saved settings: {e}");
        return;
    }
    s.horizon.save(dir);
}

/// Goes back to the saved settings, as far as the graphics screen edits them. Whether
/// there were any.
pub fn load(s: &mut Settings) -> bool {
    if !saved_exists() {
        return false;
    }
    let saved = Settings::load(&saved_options());
    // Servers named for BlueMap aren't graphics; they live in their own file.
    let servers = s.horizon.blue_map.servers.clone();
    crate::graphics_ui::copy_screen_settings(&saved, s);
    s.horizon.blue_map.servers = servers;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_level_costs_more_than_the_one_before() {
        assert_eq!(best_performance().cost, 1.0);
        for pair in LADDER.windows(2) {
            assert!(pair[1].cost < pair[0].cost, "{pair:?}");
        }
    }

    #[test]
    fn a_target_gets_the_best_level_that_should_reach_it() {
        let top = LADDER[LADDER.len() - 1];
        assert_eq!(level_for(30, 2000.0), top);
        // Out of reach: the most frames there are.
        assert_eq!(level_for(5000, 2000.0), LADDER[0]);
        assert_eq!(level_for(2000, 2000.0), LADDER[0]);
        for target in [100, 500, 1000, 1500] {
            let got = level_for(target, 2000.0);
            assert!(2000.0 * got.cost >= target as f32, "{target}: {got:?}");
        }
    }

    #[test]
    fn the_slider_spans_the_range_and_comes_back() {
        assert_eq!(target_between(0.0, 30, 2000), 30);
        assert_eq!(target_between(1.0, 30, 2000), 2000);
        let middle = target_between(0.5, 30, 2000);
        assert!(middle > 200 && middle < 300, "{middle}");
        for fps in [30, 60, 240, 1000, 2000] {
            let back = target_between(position_between(fps, 30, 2000), 30, 2000);
            assert!(back.abs_diff(fps) <= 10, "{fps} came back as {back}");
        }
        assert_eq!(position_between(0, 30, 2000), 0.5);
    }

    #[test]
    fn slower_parts_guess_lower() {
        let hw = |gpu, cores| Hardware {
            gpu,
            threads: cores * 2,
            cores,
            memory_gib: 16,
        };
        assert!(guess(hw(Gpu::Discrete, 8)) > guess(hw(Gpu::Discrete, 4)));
        assert!(guess(hw(Gpu::Discrete, 4)) > guess(hw(Gpu::Integrated, 8)));
        assert!(guess(hw(Gpu::Integrated, 8)) > guess(hw(Gpu::Software, 8)));
    }

    #[test]
    fn it_sets_what_it_picked() {
        let mut s = Settings::default();
        apply(best_performance(), &mut s);
        assert_eq!(s.horizon.preset, Preset::Performance);
        assert_eq!(s.simulation_distance, 6);
        assert!(s.horizon.dynamic_hud);
    }
}
