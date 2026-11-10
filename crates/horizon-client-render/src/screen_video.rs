//! Cottleta's Settings: the player's own videos as backgrounds (`screen_picture`), decoded on a
//! thread of their own a frame ahead of the one showing. An animated WebP decodes here; any
//! other video (MP4, WebM, MOV, MKV, GIF) through the FFmpeg program, from the pictures folder
//! or the system's path. Frames wait in a channel of one: while the video isn't on the screen
//! nobody takes them and decoding stops.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// One frame: its size, its pixels (RGBA, sRGB) and how long it shows.
pub struct Frame {
    pub size: [u32; 2],
    pub rgba: Vec<u8>,
    pub shows_for: Duration,
}

/// The video file extensions read through FFmpeg.
pub const FFMPEG_EXTENSIONS: [&str; 6] = ["mp4", "webm", "mov", "mkv", "gif", "avi"];

/// Frames a second asked of FFmpeg: smooth enough for a background, half the uploads of 60.
const FFMPEG_FPS: u32 = 30;
/// The largest frame asked of FFmpeg; bigger videos are scaled down to fit.
const MAX_SIZE: [u32; 2] = [1920, 1080];

/// Whether `bytes` are an animated WebP.
pub fn is_animated_webp(bytes: &[u8]) -> bool {
    image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).is_ok_and(|d| d.is_animated())
}

/// Plays the animated WebP `bytes` over and over, from a thread.
pub fn play_webp(bytes: Vec<u8>) -> Option<mpsc::Receiver<Frame>> {
    let mut decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    if !decoder.is_animated() {
        return None;
    }
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("Picture video".into())
        .spawn(move || {
            let (w, h) = decoder.dimensions();
            let alpha = decoder.has_alpha();
            let Some(len) = decoder.output_buffer_size() else {
                return;
            };
            let mut buf = vec![0u8; len];
            loop {
                let ms = match decoder.read_frame(&mut buf) {
                    Ok(ms) => ms,
                    Err(image_webp::DecodingError::NoMoreFrames) => {
                        decoder.reset_animation();
                        continue;
                    }
                    Err(_) => return,
                };
                let rgba = if alpha {
                    buf.clone()
                } else {
                    buf.chunks_exact(3)
                        .flat_map(|p| [p[0], p[1], p[2], 255])
                        .collect()
                };
                let frame = Frame {
                    size: [w, h],
                    rgba,
                    // Browsers show 0 and 10 ms frames for 100 ms.
                    shows_for: Duration::from_millis(if ms <= 10 { 100 } else { ms as u64 }),
                };
                if tx.send(frame).is_err() {
                    return;
                }
            }
        })
        .ok()?;
    Some(rx)
}

/// The FFmpeg program: `ffmpeg` beside the player's pictures, else the system's.
fn ffmpeg(pictures: &Path) -> PathBuf {
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let local = pictures.join(name);
    if local.is_file() {
        local
    } else {
        PathBuf::from(name)
    }
}

/// A command for FFmpeg without a console window of its own.
fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// The video's size from FFmpeg's description of the file (`Stream #0:0: Video: h264 ...,
/// 1920x1080 ...`).
fn probe_size(program: &Path, file: &Path) -> Option<[u32; 2]> {
    let out = command(program)
        .args(["-hide_banner", "-i"])
        .arg(file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    video_size(&String::from_utf8_lossy(&out.stderr))
}

/// The first `WxH` on the first video stream's line of FFmpeg's description.
fn video_size(description: &str) -> Option<[u32; 2]> {
    let line = description.lines().find(|l| l.contains("Video:"))?;
    line.split(|c: char| c == ',' || c.is_whitespace())
        .filter_map(|word| {
            let (w, h) = word.split_once('x')?;
            Some([w.parse().ok()?, h.parse().ok()?])
        })
        .find(|[w, h]: &[u32; 2]| *w > 0 && *h > 0)
}

/// `size` scaled down to fit `MAX_SIZE`, both sides even.
fn fitted(size: [u32; 2]) -> [u32; 2] {
    let k = (MAX_SIZE[0] as f32 / size[0] as f32)
        .min(MAX_SIZE[1] as f32 / size[1] as f32)
        .min(1.0);
    let even = |v: f32| ((v / 2.0).round() as u32 * 2).max(2);
    [even(size[0] as f32 * k), even(size[1] as f32 * k)]
}

/// Ends FFmpeg with the thread reading it.
struct Kill(Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Plays the video `file` over and over through FFmpeg, from a thread. None when FFmpeg is
/// missing or can't read it.
pub fn play_ffmpeg(file: &Path, pictures: &Path) -> Option<mpsc::Receiver<Frame>> {
    let program = ffmpeg(pictures);
    let size = fitted(probe_size(&program, file)?);
    let child = command(&program)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-stream_loop",
            "-1",
            "-i",
        ])
        .arg(file)
        .args([
            "-an",
            "-vf",
            &format!("fps={FFMPEG_FPS},scale={}:{}", size[0], size[1]),
            "-pix_fmt",
            "rgba",
            "-f",
            "rawvideo",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("Picture video".into())
        .spawn(move || {
            let mut child = Kill(child);
            let Some(mut out) = child.0.stdout.take() else {
                return;
            };
            let len = 4 * size[0] as usize * size[1] as usize;
            loop {
                let mut rgba = vec![0u8; len];
                if out.read_exact(&mut rgba).is_err() {
                    return;
                }
                let frame = Frame {
                    size,
                    rgba,
                    shows_for: Duration::from_secs(1) / FFMPEG_FPS,
                };
                if tx.send(frame).is_err() {
                    return;
                }
            }
        })
        .ok()?;
    Some(rx)
}

#[cfg(test)]
mod tests {
    use super::{fitted, video_size};

    #[test]
    fn the_size_is_read_from_ffmpegs_description() {
        let text = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'a.mp4':\n  Duration: 00:00:05.00\n  \
            Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(progressive), \
            1280x720 [SAR 1:1 DAR 16:9], 1000 kb/s, 30 fps\n";
        assert_eq!(video_size(text), Some([1280, 720]));
        assert_eq!(video_size("Stream #0:0: Audio: aac, 44100 Hz"), None);
    }

    #[test]
    fn big_videos_are_scaled_to_fit_with_even_sides() {
        assert_eq!(fitted([3840, 2160]), [1920, 1080]);
        assert_eq!(fitted([640, 360]), [640, 360]);
        assert_eq!(fitted([1081, 1081]), [1080, 1080]);
    }
}
