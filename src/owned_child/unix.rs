use std::io;
use std::process::Command;
use std::sync::{Arc, Mutex};
use nix::errno::Errno::ESRCH;
use nix::libc::pid_t;
use nix::sys::signal;
use nix::sys::signal::SIGKILL;
use nix::sys::wait::{Id, waitid, waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use crate::owned_child::ExitStatus;

#[derive(Debug)]
pub(super) struct ChildHandle {
    pid: Mutex<Option<Pid>>,
}

impl ChildHandle {
    fn new(pid: Pid) -> Self {
        ChildHandle {
            pid: Mutex::new(Some(pid)),
        }
    }

    pub(super) fn try_kill(&self) -> nix::Result<()> {
        let pid = self.pid.lock().unwrap();
        if let Some(pid) = *pid {
            // While we hold the lock and the inner value is not None
            // the process has not been waited for yet
            unsafe { try_kill(pid)? }
        }
        // Ensure that the Mutex is still locked while calling `try_kill`
        drop(pid);
        Ok(())
    }

    // /// If `may_be_running` returns `false` then the child process has exited
    // /// and this handle is now useless.
    // ///
    // /// A return value of `true` does **not** however guarantee
    // /// that the child process is still running - it might have just exited.
    // ///
    // /// This function does not block, even when the inner Mutex is locked.
    // pub(super) fn may_be_running(&self) -> bool {
    //     match self.pid.try_lock() {
    //         Err(_) => true,
    //         Ok(value) => value.is_some(),
    //     }
    // }
}

pub(super) struct OwnedChild {
    handle: Arc<ChildHandle>,
}

impl OwnedChild {
    /// # Safety
    ///
    /// The caller must guarantee that the PID has not been waited for
    /// and will not be waited for in the future
    unsafe fn from_nix_pid(pid: Pid) -> Self {
        OwnedChild { handle: Arc::new(ChildHandle::new(pid)) }
    }

    fn to_nix_pid(&self) -> Pid {
        self.handle.pid.lock().unwrap().expect("handle.pid should not be None until OwnedPid drops")
    }

    pub(super) fn spawn_command(command: &mut Command) -> io::Result<Self> {
        let child = command.spawn()?;
        let pid = Pid::from_raw(child.id() as pid_t);
        // The PID is still valid because we have not waited for the child
        // and because we are not returning it, no other code will
        let owned_child = unsafe { Self::from_nix_pid(pid) };
        Ok(owned_child)
    }

    pub(super) fn wait(self) -> io::Result<ExitStatus> {
        let wait_status = waitid(
            Id::Pid(self.to_nix_pid()),
            WaitPidFlag::WEXITED | WaitPidFlag::WSTOPPED | WaitPidFlag::WNOWAIT
        )?;
        Ok(match wait_status {
            WaitStatus::Exited(_, exit_code) => ExitStatus::ExitCode(exit_code),
            WaitStatus::Signaled(_, signal, _) => ExitStatus::Signalled(signal.as_str()),
            other => panic!("Received unexpected exit status when waiting for child: {:?}", other)
        })
    }

    pub(super) fn get_handle_arc(&self) -> &Arc<ChildHandle> {
        &self.handle
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        // To avoid killing a process which reused our PID by future calls to `ProcessHandle::kill()`,
        // we set `handle.pid` to None by using `Option::take()`.
        let pid = self.handle.pid.lock()
            // even if another thread poisons the Mutex, we should still call waitpid
            .unwrap_or_else(|err| err.into_inner())
            .take().expect("handle.pid should not be None until OwnedPid drops");

        // We only wait in the drop handler, so it's safe
        unsafe { try_kill(pid) }.unwrap();

        // `waitpid` without the `WNOWAIT` flag lets the OS clean resources of the child
        // and lets the PID by reused by another process.
        waitpid(pid, None).unwrap();
    }
}

/// # Safety
///
/// The caller must ensure `pid` must not have been waited for yet
unsafe fn try_kill(pid: Pid) -> nix::Result<()> {
    let result = signal::kill(pid, Some(SIGKILL));
    // ESRCH means the process does not exist
    // or has terminated execution but has not been waited for.
    // This might mean it has already been killed.
    if let Err(ESRCH) = result {
        return Ok(());
    }
    result
}
