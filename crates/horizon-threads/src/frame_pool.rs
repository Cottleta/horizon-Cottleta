//! A few threads kept ready for work inside a frame: starting threads each frame costs
//! more than the work can save, and the wake-up shows in the slowest frames. The caller
//! takes part and returns when all of it is done, so a job can borrow from the caller's
//! stack. One job at a time: another caller meanwhile does its parts alone.
use std::any::Any;
use std::sync::{
    Condvar, Mutex, OnceLock,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

/// How long a free helper watches for the next job before it sleeps: at high frame rates
/// the next one is that close, and sleeping and waking are system calls on both sides.
const WATCH: Duration = Duration::from_micros(100);

#[derive(Clone, Copy)]
struct Job(*const (dyn Fn(usize) + Sync));
// SAFETY: the pointee is `Sync`, and `run` keeps it alive until every part has finished;
// parts are only claimed through `claim`, which a finished job's epoch can't pass.
unsafe impl Send for Job {}

#[derive(Default)]
struct Shared {
    /// The job's epoch above the next part to claim: a helper that read an older job can't
    /// claim a part of this one.
    claim: AtomicU64,
    epoch: AtomicU64,
    done: AtomicUsize,
    current: Mutex<Option<(u64, Job, usize)>>,
    sleepers: AtomicUsize,
    wake: Condvar,
    panic: Mutex<Option<Box<dyn Any + Send>>>,
    running: Mutex<()>,
}

const INDEX_BITS: u32 = 16;

impl Shared {
    fn work(&self, epoch: u64, job: Job, parts: usize) {
        loop {
            let claim = self.claim.load(Ordering::Acquire);
            let index = (claim & ((1 << INDEX_BITS) - 1)) as usize;
            if claim >> INDEX_BITS != epoch || index >= parts {
                return;
            }
            if self
                .claim
                .compare_exchange_weak(claim, claim + 1, Ordering::AcqRel, Ordering::Relaxed)
                .is_err()
            {
                continue;
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // SAFETY: a part was claimed under this job's epoch, so `run` is still
                // waiting for it and the closure lives.
                (unsafe { &*job.0 })(index);
            }));
            if let Err(panic) = result {
                *self.panic.lock().unwrap_or_else(|e| e.into_inner()) = Some(panic);
            }
            self.done.fetch_add(1, Ordering::AcqRel);
        }
    }
    fn helper(&self) {
        let mut seen = 0;
        loop {
            let started = Instant::now();
            while self.epoch.load(Ordering::Acquire) == seen && started.elapsed() < WATCH {
                std::hint::spin_loop();
            }
            let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
            while self.epoch.load(Ordering::Acquire) == seen {
                self.sleepers.fetch_add(1, Ordering::AcqRel);
                current = self.wake.wait(current).unwrap_or_else(|e| e.into_inner());
                self.sleepers.fetch_sub(1, Ordering::AcqRel);
            }
            seen = self.epoch.load(Ordering::Acquire);
            let job = *current;
            drop(current);
            if let Some((epoch, job, parts)) = job {
                self.work(epoch, job, parts);
            }
        }
    }
}

/// Threads a job can use, the caller among them.
pub fn threads() -> usize {
    (crate::worker_count() / 2).clamp(1, 4)
}

fn shared() -> &'static Shared {
    static SHARED: OnceLock<&'static Shared> = OnceLock::new();
    SHARED.get_or_init(|| {
        let shared: &'static Shared = Box::leak(Box::default());
        for _ in 1..threads() {
            // Without one, the caller does more of the parts.
            let _ = std::thread::Builder::new()
                .name("Frame helper".into())
                .spawn(move || {
                    crate::avoid_interrupt_core();
                    shared.helper();
                });
        }
        shared
    })
}

/// Runs `job(0..parts)`, each part once, and returns when all have finished. A part's panic
/// continues here.
pub fn run(parts: usize, job: &(dyn Fn(usize) + Sync)) {
    let shared = shared();
    let Ok(_running) = shared.running.try_lock() else {
        (0..parts).for_each(job);
        return;
    };
    // SAFETY: only the lifetime goes: this function doesn't return while a part may run.
    let erased: *const (dyn Fn(usize) + Sync + 'static) = unsafe { std::mem::transmute(job) };
    let job = Job(erased);
    let epoch = shared.epoch.load(Ordering::Acquire) + 1;
    shared.done.store(0, Ordering::Release);
    {
        let mut current = shared.current.lock().unwrap_or_else(|e| e.into_inner());
        *current = Some((epoch, job, parts));
        shared.claim.store(epoch << INDEX_BITS, Ordering::Release);
        shared.epoch.store(epoch, Ordering::Release);
    }
    if shared.sleepers.load(Ordering::Acquire) > 0 {
        shared.wake.notify_all();
    }
    shared.work(epoch, job, parts);
    while shared.done.load(Ordering::Acquire) < parts {
        std::hint::spin_loop();
    }
    if let Some(panic) = shared.panic.lock().unwrap_or_else(|e| e.into_inner()).take() {
        std::panic::resume_unwind(panic);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_part_runs_once_and_borrows_stay_valid() {
        for round in 0..2000 {
            let parts = round % 7 + 1;
            let hits: Vec<AtomicUsize> = (0..parts).map(|_| AtomicUsize::new(0)).collect();
            run(parts, &|i| {
                hits[i].fetch_add(1, Ordering::Relaxed);
            });
            assert!(hits.iter().all(|h| h.load(Ordering::Relaxed) == 1));
        }
    }

    #[test]
    fn a_part_that_panics_continues_in_the_caller_and_the_pool_goes_on() {
        let caught = std::panic::catch_unwind(|| run(4, &|i| assert_ne!(i, 2)));
        assert!(caught.is_err());
        let hits = AtomicUsize::new(0);
        run(4, &|_| {
            hits.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(hits.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn jobs_after_a_sleep_still_run() {
        std::thread::sleep(WATCH * 20);
        let hits = AtomicUsize::new(0);
        run(3, &|_| {
            hits.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(hits.load(Ordering::Relaxed), 3);
    }
}
