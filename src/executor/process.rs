use std::ffi::{CString, OsString};
use std::fs::File;
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null_mut};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use nix::errno::Errno::ESRCH;
use nix::libc::{c_char, c_int, pid_t, posix_spawn_file_actions_adddup2, posix_spawn_file_actions_destroy, posix_spawn_file_actions_init, posix_spawn_file_actions_t, posix_spawnattr_destroy, posix_spawnattr_init, posix_spawnattr_setflags, posix_spawnattr_t, STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO};
use nix::sys::signal;
use nix::sys::signal::SIGKILL;
use nix::sys::wait::{Id, waitid, waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{Pid};
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
    unsafe fn from_nix_pid(pid: Pid) -> Self {
        OwnedPid { handle: ProcessHandle::new(pid) }
    }

    fn to_nix_pid(&self) -> Pid {
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

        // We only wait in the drop handler, so it's ok
        unsafe { try_kill(pid); }

        // `waitpid` without the `WNOWAIT` flag lets the OS clean resources of the child
        // and lets the PID by reused by another process.
        waitpid(pid, None).unwrap();
    }
}

// Based on std::process::Command::spawn() Unix implementation:
// https://github.com/rust-lang/rust/blob/8c7c151a7a03d92cc5c75c49aa82a658ec1fe4ff/library/std/src/sys/pal/unix/process/process_unix.rs#L446
fn posix_spawn(executable_file_path: &Path, stdin: &File, stdout: &File, stderr: &File) -> io::Result<OwnedPid> {
    struct PosixSpawnFileActions<'a>(&'a mut MaybeUninit<posix_spawn_file_actions_t>);

    impl Drop for PosixSpawnFileActions<'_> {
        fn drop(&mut self) {
            unsafe {
                posix_spawn_file_actions_destroy(self.0.as_mut_ptr());
            }
        }
    }

    struct PosixSpawnattr<'a>(&'a mut MaybeUninit<posix_spawnattr_t>);

    impl Drop for PosixSpawnattr<'_> {
        fn drop(&mut self) {
            unsafe {
                posix_spawnattr_destroy(self.0.as_mut_ptr());
            }
        }
    }

    // Also copied from the std implementation
    pub fn cvt_nz(error: c_int) -> io::Result<()> {
        if error == 0 { Ok(()) } else { Err(io::Error::from_raw_os_error(error)) }
    }

    let filename = executable_file_path.file_name().map(|e| e.to_owned()).unwrap_or(OsString::from("program"));
    let executable_file_path = CString::new(executable_file_path.as_os_str().as_bytes()).unwrap();

    unsafe {
        let mut attrs = MaybeUninit::uninit();
        cvt_nz(posix_spawnattr_init(attrs.as_mut_ptr()))?;
        let attrs = PosixSpawnattr(&mut attrs);

        let mut file_actions = MaybeUninit::uninit();
        cvt_nz(posix_spawn_file_actions_init(file_actions.as_mut_ptr()))?;
        let file_actions = PosixSpawnFileActions(&mut file_actions);

        cvt_nz(posix_spawn_file_actions_adddup2(
            file_actions.0.as_mut_ptr(),
            stdin.as_raw_fd(),
            STDIN_FILENO,
        ))?;
        cvt_nz(posix_spawn_file_actions_adddup2(
            file_actions.0.as_mut_ptr(),
            stdout.as_raw_fd(),
            STDOUT_FILENO,
        ))?;
        cvt_nz(posix_spawn_file_actions_adddup2(
            file_actions.0.as_mut_ptr(),
            stderr.as_raw_fd(),
            STDERR_FILENO,
        ))?;

        cvt_nz(posix_spawnattr_setflags(attrs.0.as_mut_ptr(), 0))?;

        let mut pid: pid_t = 0;

        // [Dominik]: I wrote it, it doesn't crash, but I have no idea what it does.
        // Also, it might leak??
        let argv: [*mut c_char; 2] = [CString::new(filename.as_bytes()).unwrap().into_raw(), null_mut()];
        let envp: [*mut c_char; 1] = [null_mut()];

        // [Dominik]: Same with this line
        cvt_nz(nix::libc::posix_spawn(
            &mut pid,
            executable_file_path.as_ptr(),
            file_actions.0.as_ptr(),
            attrs.0.as_ptr(),
            argv.as_ptr(),
            envp.as_ptr(),
        ))?;

        Ok(OwnedPid::from_nix_pid(Pid::from_raw(pid)))
    }
}

// Now that my posix_spawn() returns OwnedPid,
// this function could be split into separate start and wait functions.
pub(crate) fn start_and_wait(
    executable_file_path: &Path,
    stdin: &File,
    stdout: &File,
    stderr: &File,
    before_wait: impl FnOnce(&ProcessHandle)
) -> (ExecutionMetrics, Result<(), ExecutionError>) {
    // TODO: Remove unwrap
    let pid = posix_spawn(executable_file_path, stdin, stdout, stderr).unwrap();
    after_fork_parent(pid, before_wait)
    // let executable_file_path = CString::new(executable_file_path.as_os_str().as_bytes()).unwrap();
    //
    // match unsafe { fork() } {
    //     Ok(ForkResult::Parent { child: pid, .. }) => {
    //         // println!("Continuing execution in parent process, new child has pid: {}", pid);
    //         after_fork_parent(unsafe { OwnedPid::from_raw(pid) }, before_wait)
    //     },
    //     Ok(ForkResult::Child) => {
    //         // Only async-signal-safe calls are available here.
    //         // memory allocation is not async-signal-safe,
    //         // so the calls we can do are very limited.
    //         // See Safety section in `fork()`
    //
    //         trait ResultExt<T> {
    //             fn or_exit(self) -> T;
    //         }
    //
    //         impl<T, E: Sized> ResultExt<T> for Result<T, E> {
    //             fn or_exit(self) -> T {
    //                 self.unwrap_or_else(|_| {
    //                     // TODO: Save error in a provided location?
    //                     //       Would this require any IPC methods?
    //                     // exit() is not async-signal-safe.
    //                     // _exit() does not run clean up actions like flushing buffers,
    //                     // but that's what we want to happen.
    //                     unsafe { _exit(1); }
    //                 })
    //             }
    //         }
    //
    //         // Unsafe to use `println!` (or `unwrap`) here. See Safety.
    //         // write(std::io::stdout().as_raw_fd(), "I'm a new child process\n".as_bytes()).ok();
    //         let empty_list: &[&CStr] = &[];
    //         dup2(stdin.as_raw_fd(), STDIN_FILENO).or_exit();
    //         dup2(stdout.as_raw_fd(), STDOUT_FILENO).or_exit();
    //         dup2(stderr.as_raw_fd(), STDERR_FILENO).or_exit();
    //         execve(&executable_file_path, &empty_list, &empty_list).or_exit();
    //         unreachable!();
    //     }
    //     Err(_) => panic!("Fork failed"),
    // }
}

/// # Safety
///
/// The caller must ensure `pid` must not have been waited for yet
unsafe fn try_kill(pid: Pid) {
    match signal::kill(pid, Some(SIGKILL)) {
        Ok(()) => {}
        // ESRCH means the process does not exist
        // or has terminated execution but has not been waited for.
        // This might mean it has already been killed.
        Err(ESRCH) => {}
        Err(errno) => panic!("kill syscall failed with errno {}", errno),
    }
}

fn after_fork_parent(pid: OwnedPid, before_wait: impl FnOnce(&ProcessHandle)) -> (ExecutionMetrics, Result<(), ExecutionError>) {
    let start_time = Instant::now();
    before_wait(&pid.handle);

    let wait_status = waitid(Id::Pid(pid.to_nix_pid()), WaitPidFlag::WEXITED | WaitPidFlag::WSTOPPED | WaitPidFlag::WNOWAIT).unwrap();
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
