//! Measures this PC at each auto settings level, from inside the game: a temporary world, one
//! flight per level through the same control commands as `benchmark`. The player's settings wait
//! in a backup, and come back when the test ends, is stopped (leaving the world's screen) or at
//! the next start after a crash.

use std::path::Path;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::auto_settings::LEVELS;
use crate::benchmark::{Control, wait_loaded};
use crate::control::Request;

const WORLD: &str = "horizon-auto-test";
/// Written into the world so a test only ever replaces or removes its own world.
const MARKER: &str = "horizon-auto-test.txt";
const SEED: i64 = 1234;
const SETTLE: Duration = Duration::from_secs(2);
const FLIGHT_SECONDS: f64 = 8.0;
/// Over land, from `benchmark`'s forest scene.
const FROM: [f64; 3] = [-590.0, 110.0, 72.0];
const TO: [f64; 3] = [-590.0, 110.0, 272.0];

pub fn start(requests: Sender<Request>, game_dir: std::path::PathBuf) {
    thread::Builder::new()
        .name("horizon-auto-test".into())
        .spawn(move || {
            let control = Control(requests);
            match run(&control, &game_dir) {
                Ok(fps) => {
                    log::info!("Auto test: {fps:?}");
                    crate::auto_settings::note_tested(fps);
                }
                Err(e) => {
                    log::warn!("Auto test stopped: {e}");
                    crate::auto_settings::note_test_stopped();
                }
            }
            if let Err(e) = control.call("auto-end") {
                log::warn!("Auto test: {e}");
            }
            // Closing saves the world on a worker: only then can it go.
            for _ in 0..1200 {
                if control.status()["screen"] == "title" {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            remove_world(&game_dir);
            let _ = control.call("auto-show");
        })
        .expect("spawn the auto test thread");
}

fn world(game_dir: &Path) -> std::path::PathBuf {
    game_dir.join("saves").join(WORLD)
}

/// Never a world the test didn't create.
fn remove_world(game_dir: &Path) {
    let world = world(game_dir);
    if world.join(MARKER).exists() {
        let _ = std::fs::remove_dir_all(world);
    }
}

fn still_playing(control: &Control) -> Result<(), String> {
    match control.status()["screen"].as_str() {
        Some("playing") => Ok(()),
        other => Err(format!("left the world's screen: {other:?}")),
    }
}

fn run(control: &Control, game_dir: &Path) -> Result<[f32; LEVELS], String> {
    remove_world(game_dir);
    if world(game_dir).exists() {
        return Err(format!("{WORLD} exists and isn't the test's"));
    }
    // The mark comes first: after a crash, the next test can still remove this world.
    std::fs::create_dir_all(world(game_dir)).map_err(|e| e.to_string())?;
    std::fs::write(world(game_dir).join(MARKER), "Removed after the test.\n")
        .map_err(|e| e.to_string())?;
    control.call(&format!("open {WORLD} {SEED} creative"))?;
    wait_loaded(control, |_| true)?;
    let [x, y, z] = FROM;
    let [ex, ey, ez] = TO;
    let near = |p: &Value| {
        p.as_array().is_some_and(|p| {
            p.len() == 3
                && p.iter()
                    .zip(FROM)
                    .map(|(a, b)| (a.as_f64().unwrap_or(f64::MAX) - b).powi(2))
                    .sum::<f64>()
                    < 4.0
        })
    };
    control.call("time 6000")?;
    let mut fps = [0.0; LEVELS];
    for (level, fps) in fps.iter_mut().enumerate() {
        control.call(&format!("auto-level {level}"))?;
        control.call(&format!("tp {x} {y} {z}"))?;
        control.call(&format!("fly {x} {y} {z} {x} {y} {z} 0.1 15"))?;
        wait_loaded(control, near)?;
        still_playing(control)?;
        thread::sleep(SETTLE);
        let flight = control.call(&format!(
            "fly {x} {y} {z} {ex} {ey} {ez} {FLIGHT_SECONDS} 15"
        ))?;
        still_playing(control)?;
        let summary = flight.get("summary").unwrap_or(&flight);
        *fps = summary["avg_fps"].as_f64().ok_or("no frame rate")? as f32;
    }
    Ok(fps)
}
