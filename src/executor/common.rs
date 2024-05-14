use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::{Duration, Instant};
use crate::flag::Flag;
use crate::owned_child::ChildHandle;

/// Kills the `ChildHandles` passed using `Watchdog::add_handle()`
/// after `timeout_duration` or when `kill_flag` is raised.
///
/// Since the timeout duration is the same for all children,
/// we can always just wait for the next child in the queue to time out.
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

    pub(crate) fn add_handle(&self, handle: ChildHandle) {
        let timeout = Instant::now() + self.timeout_duration;
        self.sender.send(WatchdogMessage { handle, timeout }).unwrap();
    }
}