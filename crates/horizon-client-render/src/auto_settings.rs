//! Quick auto settings: what this PC's GPU, CPU and memory suggest, without testing
//! anything. Frame rate comes first: a slower part of the three picks for all of them.
use crate::graphics::Preset;
use crate::settings::Settings;
use std::sync::atomic::{AtomicU8, Ordering};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Picks {
    pub preset: Preset,
    pub render_distance: i32,
    pub simulation_distance: i32,
}

/// The settings for this hardware.
pub fn pick(hw: Hardware) -> Picks {
    let (preset, render_distance, simulation_distance) = match hw.gpu {
        Gpu::Software => (Preset::Performance, 6, 6),
        _ if hw.gpu != Gpu::Discrete || hw.cores < 6 || hw.memory_gib < 16 => {
            (Preset::Performance, 8, 6)
        }
        _ => (Preset::Balanced, 12, 8),
    };
    Picks {
        preset,
        render_distance,
        simulation_distance,
    }
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

/// Sets what `pick` chose, and the options that only save work.
pub fn apply(picks: Picks, s: &mut Settings) {
    crate::graphics::apply_preset(picks.preset, s);
    s.horizon.preset = picks.preset;
    s.render_distance = picks.render_distance;
    s.simulation_distance = picks.simulation_distance;
    s.horizon.dynamic_hud = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(gpu: Gpu, threads: usize, cores: usize, memory_gib: u64) -> Hardware {
        Hardware {
            gpu,
            threads,
            cores,
            memory_gib,
        }
    }

    #[test]
    fn the_slowest_part_decides() {
        let good = pick(hw(Gpu::Discrete, 8, 8, 32));
        assert_eq!((good.preset, good.render_distance), (Preset::Balanced, 12));
        for slower in [
            hw(Gpu::Discrete, 8, 4, 16),
            hw(Gpu::Discrete, 16, 8, 8),
            hw(Gpu::Integrated, 16, 8, 32),
            hw(Gpu::Unknown, 16, 8, 32),
        ] {
            let p = pick(slower);
            assert_eq!((p.preset, p.render_distance), (Preset::Performance, 8), "{slower:?}");
        }
        assert_eq!(pick(hw(Gpu::Software, 16, 8, 32)).render_distance, 6);
    }

    #[test]
    fn it_sets_what_it_picked() {
        let mut s = Settings::default();
        apply(pick(hw(Gpu::Discrete, 8, 4, 16)), &mut s);
        assert_eq!(s.horizon.preset, Preset::Performance);
        assert_eq!((s.render_distance, s.simulation_distance), (8, 6));
        assert!(s.horizon.dynamic_hud);
    }
}
