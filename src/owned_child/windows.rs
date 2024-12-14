use std::io;
use std::mem::MaybeUninit;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, TerminateProcess, WaitForSingleObject};
use crate::owned_child::ExitStatus;

#[derive(Debug)]
pub(super) struct ChildHandle {
    inner: OwnedHandle,
    terminated: AtomicBool,
}

impl ChildHandle {
    fn as_win_handle(&self) -> HANDLE {
        HANDLE(self.inner.as_raw_handle() as isize)
    }

    pub(super) fn try_kill(&self) -> windows::core::Result<()> {
        if self.is_useless() { return Ok(()); }
        unsafe {
            TerminateProcess(self.as_win_handle(), 1)
        }?;
        self.terminated.store(true, Relaxed);
        Ok(())
    }

    pub(super) fn is_useless(&self) -> bool {
        self.terminated.load(Relaxed)
    }
}

impl From<OwnedHandle> for ChildHandle {
    fn from(value: OwnedHandle) -> Self {
        Self {
            inner: value,
            terminated: AtomicBool::new(false),
        }
    }
}

#[derive(Debug)]
pub(super) struct OwnedChild {
    handle: Arc<ChildHandle>,
}

impl OwnedChild {
    pub(super) fn spawn_command(command: &mut Command) -> io::Result<Self> {
        let child = command.spawn()?;
        let handle: OwnedHandle = child.into();
        Ok(OwnedChild {
            handle: Arc::new(ChildHandle::from(handle)),
        })
    }

    pub(super) fn wait(self) -> io::Result<ExitStatus> {
        let exit_code = unsafe {
            let handle = self.handle.as_win_handle();
            // TODO: Use this mechanism instead of watchdog for timeout
            WaitForSingleObject(handle, INFINITE);
            let mut exit_code = MaybeUninit::<u32>::zeroed();
            GetExitCodeProcess(handle, exit_code.as_mut_ptr())?;
            exit_code.assume_init()
        };
        let result = ExitStatus::ExitCode(exit_code as i32);
        self.handle.terminated.store(true, Relaxed);
        Ok(result)
    }

    pub(super) fn get_handle_arc(&self) -> &Arc<ChildHandle> {
        &self.handle
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        self.handle.try_kill().unwrap();
    }
}