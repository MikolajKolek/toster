use std::ffi::CStr;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use nix::errno::Errno::ESRCH;
use nix::libc::{STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO};
use nix::sys::signal;
use nix::sys::signal::SIGKILL;
use nix::sys::wait::{Id, waitid, waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{dup2, fexecve, fork, ForkResult, Pid};
use crate::test_errors::{ExecutionError, ExecutionMetrics};
use crate::test_errors::ExecutionError::RuntimeError;

/// ProcessHandle is used to kill a child process from another thread.
///
/// Unlike `std::process::Child`, `ProcessHandle` implements `Sync` and `Send`,
/// so it can be shared between threads.
#[derive(Clone, Debug)]
pub(crate) struct ProcessHandle {
    pid: Arc<Mutex<Option<Pid>>>,
}

impl ProcessHandle {
    fn new(pid: Pid) -> Self {
        ProcessHandle {
            pid: Arc::new(Mutex::new(Some(pid))),
        }
    }

    /// Kills the process specified by the handle.
    /// Can be safely called multiple times and after the execution finishes.
    pub(crate) fn try_kill(&self) {
        let pid = self.pid.lock().unwrap();
        if let Some(pid) = *pid {
            // While we hold the lock and the inner value is not None
            // the process has not been waited for yet
            unsafe { try_kill(pid); }
        }
        // Ensure that the Mutex is still locked while calling `try_kill`
        drop(pid);
    }

    /// If `may_be_running` returns `false` then the child process has exited
    /// and this handle is now useless.
    ///
    /// A return value of `true` does **not** however guarantee
    /// that the child process is still running - it might have just exited.
    ///
    /// This function does not block, even when the inner Mutex is locked.
    pub(crate) fn may_be_running(&self) -> bool {
        match self.pid.try_lock() {
            Err(_) => true,
            Ok(value) => value.is_some(),
        }
    }
}

/// Kills and waits for the inner PID on drop,
/// to release resources and let the PID be reused by another process
///
/// The PID is guaranteed to be valid while the instance of this struct is in scope
///
/// This struct is necessary in order for the child process to be waited for
/// when a panic causes unwinding
struct OwnedPid {
    handle: ProcessHandle,
}

impl OwnedPid {
    /// # Safety
    ///
    /// The caller must guarantee that the PID has not been waited for
    /// and will not be waited for in the future
    unsafe fn from_raw(pid: Pid) -> Self {
        OwnedPid { handle: ProcessHandle::new(pid) }
    }

    fn to_raw(&self) -> Pid {
        self.handle.pid.lock().unwrap().expect("handle.pid should not be None until OwnedPid drops")
    }
}

impl Drop for OwnedPid {
    fn drop(&mut self) {
        // To avoid killing a process which reused our PID by future calls to `ProcessHandle::kill()`,
        // we set `handle.pid` to None by using `Option::take()`.
        let pid = self.handle.pid.lock()
            // even if another thread poisons the Mutex, we should still call waitpid
            .unwrap_or_else(|err| err.into_inner())
            .take().expect("handle.pid should not be None until OwnedPid drops");
        unsafe { try_kill(pid); }

        // `waitpid` without the `WNOWAIT` flag lets the OS clean resources of the child
        // and lets the PID by reused by another process.
        waitpid(pid, None).unwrap();
    }
}

pub(crate) fn start_and_wait(
    executable_file: &File,
    stdin: &File,
    stdout: &File,
    stderr: &File,
    before_wait: impl FnOnce(&ProcessHandle)
) -> (ExecutionMetrics, Result<(), ExecutionError>) {
    match unsafe { fork() } {
        Ok(ForkResult::Parent { child: pid, .. }) => {
            // println!("Continuing execution in parent process, new child has pid: {}", pid);
            after_fork_parent(unsafe { OwnedPid::from_raw(pid) }, before_wait)
        },
        Ok(ForkResult::Child) => {
            // Unsafe to use `println!` (or `unwrap`) here. See Safety.
            // write(std::io::stdout().as_raw_fd(), "I'm a new child process\n".as_bytes()).ok();
            let empty_list: &[&CStr] = &[];
            dup2(stdin.as_raw_fd(), STDIN_FILENO).unwrap();
            dup2(stdout.as_raw_fd(), STDOUT_FILENO).unwrap();
            dup2(stderr.as_raw_fd(), STDERR_FILENO).unwrap();
            fexecve(executable_file.as_raw_fd(), &empty_list, &empty_list).unwrap();
            unreachable!();
        }
        Err(_) => panic!("Fork failed"),
    }
}

/// # Safety
///
/// The caller must ensure `pid` must not have been waited for yet
unsafe fn try_kill(pid: Pid) {
    match signal::kill(pid, Some(SIGKILL)) {
        Ok(()) => {}
        // ESRCH means the process does not exist or has terminated execution
        // but has not been waited for.
        Err(ESRCH) => {}
        Err(errno) => panic!("kill syscall failed with errno {}", errno),
    }
}

fn after_fork_parent(pid: OwnedPid, before_wait: impl FnOnce(&ProcessHandle)) -> (ExecutionMetrics, Result<(), ExecutionError>) {
    let start_time = Instant::now();
    before_wait(&pid.handle);

    let wait_status = waitid(Id::Pid(pid.to_raw()), WaitPidFlag::WEXITED | WaitPidFlag::WSTOPPED | WaitPidFlag::WNOWAIT).unwrap();
    let result = match wait_status {
        WaitStatus::Exited(_, 0) => {
            Ok(())
        },
        WaitStatus::Exited(_, exit_code) => {
            Err(RuntimeError(format!("- the program returned a non-zero return code: {}", exit_code)))
        },
        WaitStatus::Signaled(_, signal, _) => {
            Err(RuntimeError(format!("- the process was terminated with the following error:\n{}", signal)))
        },
        other => panic!("Received unexpected wait status: {:?}", other),
    };
    let metrics = ExecutionMetrics {
        time: Some(Instant::now() - start_time),
        memory_kibibytes: None,
    };

    drop(pid);
    (metrics, result)
}
