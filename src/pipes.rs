use std::{io, thread};
use std::io::Read;
use std::process::Stdio;
use std::thread::JoinHandle;
use os_pipe::PipeWriter;
use crate::test_errors::ExecutionError;
#[cfg(all(unix))]
use std::os::fd::{AsRawFd, RawFd};

pub struct BufferedPipe {
    writer: Option<PipeWriter>,
    handle: JoinHandle<io::Result<Vec<u8>>>,
}

impl BufferedPipe {
    pub(crate) fn create() -> io::Result<Self> {
        let (mut reader, writer) = os_pipe::pipe()?;
        let handle = thread::spawn(move || -> io::Result<Vec<u8>> {
            let mut buffer: Vec<u8> = Vec::new();
            reader.read_to_end(&mut buffer)?;
            return Ok(buffer)
        });
        Ok(BufferedPipe {
            writer: Some(writer),
            handle,
        })
    }

    pub(crate) fn get_stdio(&mut self) -> Stdio {
        Stdio::from(self.writer.take().expect("Buffered pipe writer was accessed more than once"))
    }

    #[cfg(all(unix))]
    pub(crate) fn get_raw_fd(&self) -> RawFd {
        self.writer.as_ref().expect("Buffered pipe writer was accessed more than once").as_raw_fd()
    }

    pub(crate) fn join(mut self) -> Result<String, ExecutionError> {
        // Close the writer if we are still the owner
        // Otherwise the reader thread would never return
        drop(self.writer.take());

        let Ok(output) = self.handle.join().expect("Output reader thread panicked") else {
            return Err(ExecutionError::PipeError);
        };
        let Ok(output) = String::from_utf8(output) else {
            return Err(ExecutionError::OutputNotUtf8)
        };
        Ok(output)
    }
}
