#[cfg_attr(unix, path = "unix.rs")]
#[cfg_attr(windows, path = "windows.rs")]
mod imp;

use std::error::Error;
use std::io;
use std::process::Command;
use std::sync::Arc;
use static_assertions::assert_impl_all;

pub(crate) enum ExitStatus {
    ExitCode(i32),
    #[allow(unused)] // On Windows the process always terminates with an exit code.
    Signalled(&'static str),
}

/// ChildHandle is used to kill a child process from another thread.
///
/// Unlike `std::process::Child`, `ChildHandle` implements `Sync` and `Send`,
/// so it can be shared between threads.
#[derive(Clone)]
pub(crate) struct ChildHandle {
    inner: Arc<imp::ChildHandle>,
}
assert_impl_all!(ChildHandle: Sync, Send);

impl ChildHandle {
    /// Kills the process specified by the handle if it didn't terminate yet.
    ///
    /// Can be safely called multiple times and after the execution finishes.
    ///
    /// Can be called and does **not** block if `OwnedChild::wait_for_status`
    /// is blocking another thread.
    pub(crate) fn try_kill(self) -> Result<(), impl Error> {
        self.inner.try_kill()
    }

    /// Returns `true` if the process has already finished execution,
    /// either exiting normally or after being killed.
    ///
    /// A return value of `false` does **not** however mean that the
    /// process is still running.
    pub(crate) fn is_useless(&self) -> bool {
        self.inner.is_useless()
    }
}

/// Kills the process and releases resources on drop.
///
/// On Unix this is done by calling the `waitid` syscall on drop - the PID is guaranteed
/// to be valid while the instance of this struct is in scope.
///
/// This struct is necessary in order for the resources consumed by the child process to be released
/// when a panic causes unwinding.
pub(crate) struct OwnedChild {
    inner: imp::OwnedChild,
}

impl OwnedChild {
    pub(crate) fn wait(self) -> io::Result<ExitStatus> {
        self.inner.wait()
    }

    pub(crate) fn get_handle(&self) -> ChildHandle {
        ChildHandle { inner: self.inner.get_handle_arc().clone() }
    }
}
assert_impl_all!(ChildHandle: Sync, Send);

pub(crate) trait CommandExt {
    fn spawn_owned(&mut self) -> io::Result<OwnedChild>;
}

impl CommandExt for Command {
    fn spawn_owned(&mut self) -> io::Result<OwnedChild> {
        Ok(OwnedChild {
            inner: imp::OwnedChild::spawn_command(self)?,
        })
    }
}
