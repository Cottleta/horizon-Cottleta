//! The last steps of a frame on their own thread: closing its command encoder,
//! submitting it and presenting. They take a large part of a frame, most of it in
//! the driver, and nothing on the render thread needs their result before it next
//! touches the GPU (`Renderer::settle`), so the next frame's world, entities and
//! HUD are built meanwhile.
use std::{
    sync::mpsc::{Receiver, Sender, channel},
    time::{Duration, Instant},
};

struct Job {
    encoder: wgpu::CommandEncoder,
    surface: wgpu::SurfaceTexture,
}

/// How long a frame's steps took.
pub(super) struct Done {
    pub finish: Duration,
    pub submit: Duration,
    pub present: Duration,
}

pub(super) struct Submitter {
    jobs: Sender<Job>,
    done: Receiver<Done>,
}

impl Submitter {
    pub fn start(queue: wgpu::Queue) -> Self {
        let (jobs, inbox) = channel::<Job>();
        let (outbox, done) = channel();
        let _ = std::thread::Builder::new()
            .name("Frame submit".into())
            .spawn(move || {
                for job in inbox {
                    let started = Instant::now();
                    let commands = job.encoder.finish();
                    let finish = started.elapsed();
                    let started = Instant::now();
                    queue.submit([commands]);
                    let submit = started.elapsed();
                    let started = Instant::now();
                    queue.present(job.surface);
                    let present = started.elapsed();
                    let _ = outbox.send(Done {
                        finish,
                        submit,
                        present,
                    });
                }
            });
        Self { jobs, done }
    }
    pub fn hand_over(&self, encoder: wgpu::CommandEncoder, surface: wgpu::SurfaceTexture) {
        let _ = self.jobs.send(Job { encoder, surface });
    }
    /// The frame's steps, once they are done; None if the thread is gone.
    pub fn wait(&self) -> Option<Done> {
        self.done.recv().ok()
    }
}
