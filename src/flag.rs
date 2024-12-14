use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Condvar, Mutex, WaitTimeoutResult};
use std::time::Duration;

pub(crate) struct Flag {
    is_set: AtomicBool,
    mutex: Mutex<bool>,
    condvar: Condvar,
}

impl Flag {
    pub(crate) const fn new() -> Self {
        Self {
            is_set: AtomicBool::new(false),
            mutex: Mutex::new(false),
            condvar: Condvar::new(),
        }
    }

    pub(crate) fn raise(&self) {
        self.is_set.store(true, Relaxed);
        *self.mutex.lock().unwrap() = true;
        self.condvar.notify_all();
    }

    pub(crate) fn was_set(&self) -> bool {
        self.is_set.load(Relaxed)
    }

    pub(crate) fn wait(&self) {
        let guard = self.condvar.wait_while(
            self.mutex.lock().unwrap(),
            |x| *x == false,
        ).unwrap();
        drop(guard);
    }

    pub(crate) fn wait_with_timeout(&self, duration: Duration) -> WaitTimeoutResult {
        let (guard, result) = self.condvar.wait_timeout_while(
            self.mutex.lock().unwrap(),
            duration,
            |x| *x == false,
        ).unwrap();
        drop(guard);
        result
    }
}