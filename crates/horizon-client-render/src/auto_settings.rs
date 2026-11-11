//! Auto settings: the most frames this PC can reach, or the best graphics that should
//! still reach a frame rate the player picks, and the player's own saved settings.
//!
//! Nothing is tested here. What this PC can reach comes from the frame times of play so
//! far, scaled by what each level of `LADDER` costs; before any play, from its parts.
use crate::graphics::Preset;
use crate::settings::Settings;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

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
    /// Far terrain, in chunks. It costs more than anything else here, so every level says.
    pub lod_distance: u32,
    /// The share of the first level's frame rate this one keeps.
    pub cost: f32,
}

const fn level(
    preset: Preset,
    render_distance: i32,
    simulation_distance: i32,
    lod_distance: u32,
    cost: f32,
) -> Picks {
    Picks {
        preset,
        render_distance,
        simulation_distance,
        lod_distance,
        cost,
    }
}

pub const LEVELS: usize = 5;

/// The levels from most frames to best graphics. The costs are measured over terrain in
/// single player on one fast PC (7800X3D, RX 9070): a slower GPU will likely lose more.
const LADDER: [Picks; LEVELS] = [
    level(Preset::Performance, 8, 6, 0, 1.0),
    level(Preset::Performance, 12, 6, 0, 0.91),
    level(Preset::Balanced, 12, 8, 0, 0.82),
    level(Preset::Quality, 16, 12, 256, 0.25),
    level(Preset::Ultra, 24, 12, 512, 0.17),
];

/// A level by its place, from most frames.
pub fn level_at(i: usize) -> Picks {
    LADDER[i.min(LEVELS - 1)]
}

/// The most frames this PC can reach.
pub fn best_performance() -> Picks {
    LADDER[0]
}

/// The best level expected to reach `target` at the first level's `capacity` frames a second.
fn level_for(target: u32, capacity: f32) -> Picks {
    if let Some(fps) = tested() {
        return level_tested(target, fps);
    }
    LADDER
        .into_iter()
        .rev()
        .find(|level| capacity * level.cost >= target as f32)
        .unwrap_or(LADDER[0])
}

/// The best level that reached `target` when this PC was measured.
fn level_tested(target: u32, fps: [f32; LEVELS]) -> Picks {
    (0..LEVELS)
        .rev()
        .find(|&i| fps[i] >= target as f32)
        .map_or(LADDER[0], |i| LADDER[i])
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
    if let Some(fps) = tested() {
        return fps[0].max(1.0);
    }
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
    s.horizon.lod_distance = picks.lod_distance;
    s.horizon.dynamic_hud = true;
}

/// Sets a level to measure it, uncapped; `save_for_test` keeps what to go back to.
pub fn apply_for_test(level: usize, s: &mut Settings) {
    apply(LADDER[level.min(LEVELS - 1)], s);
    s.vsync = false;
    s.max_fps = 260;
}

/// What each level reached when this PC was last measured.
static TESTED: Mutex<Option<Option<[f32; LEVELS]>>> = Mutex::new(None);

fn tested_file() -> PathBuf {
    crate::logging::game_home().join("auto-test.txt")
}

fn parse_tested(text: &str) -> Option<[f32; LEVELS]> {
    let fps: Vec<f32> = text
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let fps: [f32; LEVELS] = fps.try_into().ok()?;
    fps.iter().all(|f| f.is_finite() && *f > 0.0).then_some(fps)
}

pub fn tested() -> Option<[f32; LEVELS]> {
    // Tests don't go by this PC's results in the real game folder.
    if cfg!(test) {
        return None;
    }
    let mut tested = TESTED.lock().unwrap_or_else(|e| e.into_inner());
    *tested.get_or_insert_with(|| {
        parse_tested(&std::fs::read_to_string(tested_file()).unwrap_or_default())
    })
}

/// Keeps a finished test's frame rates; a later estimate goes by them.
pub fn note_tested(mut fps: [f32; LEVELS]) {
    // A better level can't really be faster: that's noise, and the slower reading is safer.
    for i in 1..LEVELS {
        fps[i] = fps[i].min(fps[i - 1]);
    }
    let text = fps.map(|f| format!("{f:.0}")).join(" ");
    if let Err(e) = std::fs::write(tested_file(), text) {
        log::warn!("Auto test: {e}");
    }
    *TESTED.lock().unwrap_or_else(|e| e.into_inner()) = Some(Some(fps));
    TEST_STOPPED.store(false, Ordering::Relaxed);
}

/// The last test didn't finish, this run.
static TEST_STOPPED: AtomicBool = AtomicBool::new(false);

pub fn note_test_stopped() {
    TEST_STOPPED.store(true, Ordering::Relaxed);
}

pub fn test_stopped() -> bool {
    TEST_STOPPED.load(Ordering::Relaxed)
}

/// Measuring needs the title screen: it opens a world of its own.
static CAN_TEST: AtomicBool = AtomicBool::new(false);

pub fn set_can_test(can: bool) {
    CAN_TEST.store(can, Ordering::Relaxed);
}

pub fn can_test() -> bool {
    CAN_TEST.load(Ordering::Relaxed)
}

/// Cottleta's profiler (Settings): a world is open to profile.
static CAN_PROFILE: AtomicBool = AtomicBool::new(false);
/// Seconds left of the profile running now, 0 when none runs.
static PROFILE_LEFT: AtomicU32 = AtomicU32::new(0);
/// The last profile's average FPS this run, 0 before the first.
static PROFILE_FPS: AtomicU32 = AtomicU32::new(0);

pub fn set_can_profile(can: bool) {
    CAN_PROFILE.store(can, Ordering::Relaxed);
}

pub fn can_profile() -> bool {
    CAN_PROFILE.load(Ordering::Relaxed)
}

pub fn set_profile_left(seconds: u32) {
    PROFILE_LEFT.store(seconds, Ordering::Relaxed);
}

/// Seconds left of the running profile, if one runs.
pub fn profile_left() -> Option<u32> {
    Some(PROFILE_LEFT.load(Ordering::Relaxed)).filter(|&s| s > 0)
}

pub fn note_profile_fps(fps: f64) {
    PROFILE_FPS.store(
        fps.round().clamp(1.0, f64::from(u32::MAX)) as u32,
        Ordering::Relaxed,
    );
}

pub fn last_profile_fps() -> Option<u32> {
    Some(PROFILE_FPS.load(Ordering::Relaxed)).filter(|&f| f > 0)
}

fn test_backup() -> PathBuf {
    crate::logging::game_home()
        .join("auto-test-backup")
        .join("options.txt")
}

/// Keeps the player's settings while a test changes them, also through a crash.
pub fn save_for_test(s: &Settings) {
    save_to(&test_backup(), s);
}

/// The settings from before a test, once. Whether there were any: a test that ended
/// without restoring them leaves them until the next start.
pub fn restore_after_test(s: &mut Settings) -> bool {
    let path = test_backup();
    let done = load_from(&path, s);
    if done && let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir_all(dir);
    }
    done
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
    save_to(&saved_options(), s);
}

fn save_to(path: &std::path::Path, s: &Settings) {
    let Some(dir) = path.parent() else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(path, s.serialize()))
    {
        log::warn!("Saved settings: {e}");
        return;
    }
    s.horizon.save(dir);
}

/// Goes back to the saved settings, as far as the graphics screen edits them. Whether
/// there were any.
pub fn load(s: &mut Settings) -> bool {
    load_from(&saved_options(), s)
}

fn load_from(path: &std::path::Path, s: &mut Settings) -> bool {
    if !path.exists() {
        return false;
    }
    let saved = Settings::load(path);
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
    fn a_measured_pc_goes_by_its_own_numbers() {
        let fps = [1000.0, 900.0, 300.0, 200.0, 100.0];
        assert_eq!(level_tested(250, fps), LADDER[2]);
        assert_eq!(level_tested(100, fps), LADDER[4]);
        assert_eq!(level_tested(5000, fps), LADDER[0]);
        assert_eq!(parse_tested("1000 900 300 200 100"), Some(fps));
        assert_eq!(parse_tested("1000 900"), None);
        assert_eq!(parse_tested("a b c d e"), None);
        assert_eq!(parse_tested("0 900 300 200 100"), None);
    }

    #[test]
    fn it_sets_what_it_picked() {
        let mut s = Settings::default();
        apply(best_performance(), &mut s);
        assert_eq!(s.horizon.preset, Preset::Performance);
        assert_eq!(s.simulation_distance, 6);
        assert_eq!(s.horizon.lod_distance, 0);
        assert!(s.horizon.dynamic_hud);
    }
}
