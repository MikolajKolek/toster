use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};
use crate::test_errors::{ExecutionError, ExecutionMetrics};
use wait_timeout::ChildExt;
use crate::executor::TestExecutor;
use crate::test_errors::ExecutionError::{RuntimeError, TimedOut};

#[cfg(unix)]
use crate::generic_utils::halt;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

pub(crate) struct SimpleExecutor {
    pub(crate) timeout: Duration,
    pub(crate) executable_path: PathBuf,
}

impl SimpleExecutor {
    fn map_status_code(status: &ExitStatus) -> Result<(), ExecutionError> {
        match status.code() {
            Some(0) => Ok(()),
            Some(exit_code) => {
                Err(RuntimeError(format!("- the program returned a non-zero return code: {}", exit_code)))
            },
            None => {
                #[cfg(all(unix))]
                if status.signal().expect("The program returned an invalid status code") == 2 {
                    halt();
                }

                Err(RuntimeError(format!("- the process was terminated with the following error:\n{}", status)))
            }
        }
    }

    fn wait_for_child(&self, mut child: Child) -> (ExecutionMetrics, Result<(), ExecutionError>) {
        let start_time = Instant::now();
        let status = child.wait_timeout(self.timeout).unwrap();

        match status {
            Some(status) => (
                ExecutionMetrics { time: Some(start_time.elapsed()), memory_kibibytes: None },
                SimpleExecutor::map_status_code(&status)
            ),
            None => {
                child.kill().unwrap();
                (ExecutionMetrics { time: Some(self.timeout), memory_kibibytes: None }, Err(TimedOut))
            }
        }
    }
}

impl TestExecutor for SimpleExecutor {
    fn test_to_stdio(&self, input_stdio: Stdio, output_stdio: Stdio) -> (ExecutionMetrics, Result<(), ExecutionError>) {
        let child = Command::new(&self.executable_path)
            .stdin(input_stdio)
            .stdout(output_stdio)
            .stderr(Stdio::null())
            .spawn().expect("Failed to spawn child");

        self.wait_for_child(child)
    }
}