use std::fs::{File, OpenOptions};
use std::path::{Path};
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};
use crate::test_errors::{ExecutionError, ExecutionMetrics};
use wait_timeout::ChildExt;
use crate::executor::TestExecutor;
use crate::test_errors::ExecutionError::{RuntimeError, TimedOut};

#[cfg(unix)]
use crate::generic_utils::halt;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use crate::executor::process::start_and_wait;

pub(crate) struct SimpleExecutor {
    timeout: Duration,
    executable: File,
}

impl SimpleExecutor {
    pub(crate) fn init(executable_path: &Path, timeout: Duration) -> Self {
        let executable = OpenOptions::new().read(true).write(false).open(executable_path).unwrap();
        SimpleExecutor { timeout, executable }
    }

    fn map_status_code(status: &ExitStatus) -> Result<(), ExecutionError> {
        match status.code() {
            Some(0) => Ok(()),
            Some(exit_code) => {
                Err(RuntimeError(format!("- the program returned a non-zero return code: {}", exit_code)))
            },
            None => {
                #[cfg(unix)]
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
    fn test_to_stdio(&self, input_file: &File, output_file: &File) -> (ExecutionMetrics, Result<(), ExecutionError>) {
        // TODO: Open "/dev/null" once for the whole program (does it actually matter?)
        start_and_wait(&self.executable, input_file, output_file, &File::open("/dev/null").unwrap(), |handle| {
            // let handle = handle.clone();
            // TODO: Don't spawn separate threads for each execution?
            //       If the timeout is always the same a single thread with a FIFO queue would suffice
            //       (does it actually matter? - maybe a sleeping thread is not a problem at all)
            // let timeout = self.timeout;
            // thread::spawn(move || {
            //     thread::sleep(timeout);
            //     handle.try_kill();
            // });
        })
    }
}