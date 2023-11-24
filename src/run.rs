use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::{io, thread};
use std::time::{Duration, Instant};
use wait_timeout::ChildExt;
use crate::prepare_input::TestInputSource;
use crate::test_errors::ExecutionError::{RuntimeError, TimedOut};
use crate::test_errors::{ExecutionError, ExecutionMetrics};

#[cfg(all(unix))]
use std::os::unix::process::ExitStatusExt;
use std::thread::JoinHandle;

pub(crate) struct BasicTestRunner {
    pub(crate) timeout: Duration,
    pub(crate) executable_path: PathBuf,
}

struct StreamError(io::Error);

impl From<io::Error> for StreamError {
    fn from(value: io::Error) -> Self {
        StreamError(value)
    }
}

impl BasicTestRunner {
    fn map_status_code(status: &ExitStatus) -> Result<(), ExecutionError> {
        match status.code() {
            Some(0) => Ok(()),
            Some(exit_code) => {
                Err(RuntimeError(format!("- the program returned a non-zero return code: {}", exit_code)))
            },
            None => {
                #[cfg(all(unix))]
                if cfg!(unix) && status.signal().expect("The program returned an invalid status code!") == 2 {
                    // TODO: Implement better
                    thread::sleep(Duration::from_secs(u64::MAX));
                }

                Err(RuntimeError(format!("- the process was terminated with the following error:\n{}", status.to_string())))
            }
        }
    }

    fn wait_for_child(&self, mut child: Child) -> (ExecutionMetrics, Result<(), ExecutionError>) {
        let start_time = Instant::now();
        let status = child.wait_timeout(self.timeout).unwrap();

        match status {
            Some(status) => (
                ExecutionMetrics { time: start_time.elapsed(), memory_kilobytes: None },
                BasicTestRunner::map_status_code(&status)
            ),
            None => {
                child.kill().unwrap();
                (ExecutionMetrics { time: self.timeout, memory_kilobytes: None }, Err(TimedOut))
            }
        }
    }

    fn read_output_async(stream: &mut Option<ChildStdout>) -> JoinHandle<Result<Vec<u8>, StreamError>> {
        let stream = stream.take();
        thread::spawn(|| -> Result<Vec<u8>, StreamError> {
            let mut buffer: Vec<u8> = Vec::new();
            if let Some(mut stream) = stream {
                stream.read_to_end(&mut buffer)?;
            }
            return Ok(buffer)
        })
    }

    // pub fn test_to_file(&self, input_source: &TestInputSource, output_path: &Path) -> (ExecutionMetrics, Result<(), ExecutionError>) {
    //     let child = Command::new(&self.executable_path)
    //         .stdin(input_source.get_stdin())
    //         .stdout(output_path)
    //         .stderr(Stdio::null())
    //         .spawn().expect("Failed to spawn child");
    //     self.wait_for_child(child)
    // }

    pub fn test_to_vec(&self, input_source: &TestInputSource) -> (ExecutionMetrics, Result<Vec<u8>, ExecutionError>) {
        let mut child = Command::new(&self.executable_path)
            .stdin(input_source.get_stdin())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn().expect("Failed to spawn child");
        let output_handle = BasicTestRunner::read_output_async(&mut child.stdout);

        let (metrics, result) = self.wait_for_child(child);
        let output = output_handle.join().expect("Output thread panicked");
        (metrics, result.and_then(|_| output.map_err(|_| ExecutionError::OutputStreamError)))
    }
}