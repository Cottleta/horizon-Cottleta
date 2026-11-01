//! A record of how other players move on a server's client, for comparing
//! knockback with another client: `HORIZON_ENTITY_LOG=FILE` writes one CSV
//! line per event (`ms,event,id,x,y,z`): each move and motion packet as it
//! arrives, each hurt, and every tick where the player is drawn from (`tick`)
//! and where this player is (`self`). For this player too: its attacks on a
//! player (`attack`: sprinting, attack strength ticks), the motion a server
//! gives it (`hit`) and where each server teleport finds it (`setback`). Off
//! (nothing checked but one flag) without the variable.
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

struct Log {
    start: Instant,
    out: Mutex<BufWriter<File>>,
}

fn log() -> Option<&'static Log> {
    static LOG: OnceLock<Option<Log>> = OnceLock::new();
    LOG.get_or_init(|| {
        let path = std::env::var_os("HORIZON_ENTITY_LOG")?;
        let mut out = BufWriter::new(File::create(path).ok()?);
        writeln!(out, "ms,event,id,x,y,z").ok()?;
        Some(Log {
            start: Instant::now(),
            out: Mutex::new(out),
        })
    })
    .as_ref()
}

pub(super) fn enabled() -> bool {
    log().is_some()
}

pub(super) fn record(event: &str, id: i32, v: [f64; 3]) {
    let Some(log) = log() else {
        return;
    };
    let ms = log.start.elapsed().as_secs_f64() * 1000.0;
    if let Ok(mut out) = log.out.lock() {
        let _ = writeln!(out, "{ms:.1},{event},{id},{},{},{}", v[0], v[1], v[2]);
        // Flushed each tick, so a game closed any way keeps the record.
        if event == "self" {
            let _ = out.flush();
        }
    }
}
