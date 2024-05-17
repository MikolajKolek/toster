use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::{Duration, Instant};
use crate::flag::Flag;
use crate::owned_child::ChildHandle;

/// Kills the `ChildHandles` passed using `Watchdog::add_handle()`
/// after `timeout_duration` or when `kill_flag` is raised.
///
/// The timeout duration is the same for all children and `mpsc::channel` works
/// like a FIFO queue, so we can always just wait for the next job in the queue to time out.
///
/// # Estimating the number of thread wake-ups
/// Since I [Dominik] am already having great fun writing comments for this module,
/// we can estimate the upper and lower bounds for the number of times
/// this thread is woken up.
///
/// - Let `NUM_THREADS` be the number of worker threads.
/// - Let `TIMEOUT` be the value of `timeout_duration`.
/// - Let's also assume that it takes between 0s and `START_DELAY` for a thread
///   to start a new job after the previous one finishes.
///
/// If a job is added to the queue after we have finished waiting for all the
/// pending ones and the queue is empty, then the thread wakes up twice to handle it:
/// when a new message is received and exactly the duration of `TIMEOUT` later.
/// The maximum duration between adding new jobs to the queue is equal to
/// `TIMEOUT + START_DELAY`, so the upper bound for the time between either:
/// start of one job -> start of the next job; or
/// termination of one job -> termination of the next job
/// is `TIMEOUT + START_DELAY`, so amortized `(TIMEOUT + START_DELAY) / 2`
/// between consecutive wake-ups.
/// This rate can be reached for any `NUM_THREADS`, for example:
/// - Thread one starts a job -> first job in the queue, so the thread wakes up.
/// - Other threads also start new jobs -> the jobs are added to the queue,
///   but don't wake up the thread.
/// - All jobs terminate just before `TIMEOUT`.
/// - `TIMEOUT` passes and the thread wakes up and skips waiting for all the jobs.
/// - All threads wait for `START_DELAY`.
/// - Repeat
/// This sequence takes `TIMEOUT + START_DELAY` and the thread wakes up twice,
/// so we proved that the upper bound of `(TIMEOUT + START_DELAY) / 2` can be reached.
///
/// TODO: Lower bound
pub(crate) struct Watchdog {
    sender: Sender<WatchdogMessage>,
    timeout_duration: Duration,
}

struct WatchdogMessage {
    handle: ChildHandle,
    timeout: Instant,
}

impl Watchdog {
    pub(crate) fn start(timeout_duration: Duration, kill_flag: &'static Flag) -> Self {
        let (sender, receiver) = channel::<WatchdogMessage>();
        thread::spawn(move || {
            // The thread will be alive until the moment `Watchdog` is dropped
            // (causing the channel to hung up) plus at most `timeout_duration`.
            for WatchdogMessage { handle, timeout } in receiver.iter() {
                // This check is important for reducing the number of times this thread is woken up.
                // If we are executing 1000 tests per second and the timeout is 5s,
                // then at any point there are almost 5000 tests that have finished,
                // but have not yet timed out.
                // This line lets the watchdog skip all these 5000 finished tests,
                // before waiting for the first one that's still running to time out
                // or waiting to receive a new message if the queue is empty.
                if handle.is_useless() { continue; }

                let remaining_time = timeout.checked_duration_since(Instant::now());
                if let Some(remaining_time) = remaining_time {
                    kill_flag.wait_with_timeout(remaining_time);
                }
                handle.try_kill().unwrap();
            }
        });
        Self {
            sender,
            timeout_duration,
        }
    }

    /// Starts the timeout for the handle.
    ///
    /// If `kill_flag` was raised the child will be killed almost immediately.
    pub(crate) fn add_handle(&self, handle: ChildHandle) {
        let timeout = Instant::now() + self.timeout_duration;
        self.sender.send(WatchdogMessage { handle, timeout }).unwrap();
    }
}
