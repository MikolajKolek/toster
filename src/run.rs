use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};
use wait_timeout::ChildExt;
use crate::prepare_input::TestInputSource;
use crate::test_errors::ExecutionError::{RuntimeError, TimedOut};
use crate::test_errors::{ExecutionError, ExecutionMetrics};

#[cfg(all(unix))]
use std::os::unix::process::ExitStatusExt;
#[cfg(all(unix))]
use std::thread;
use crate::pipes::BufferedPipe;

pub(crate) struct SimpleTestRunner {
    pub(crate) timeout: Duration,
    pub(crate) executable_path: PathBuf,
}

pub(crate) trait TestRunner: Sync + Send {
    fn test_to_string(&self, input_source: &TestInputSource) -> (ExecutionMetrics, Result<String, ExecutionError>);
}

impl SimpleTestRunner {
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
                ExecutionMetrics { time: Some(start_time.elapsed()), memory_kilobytes: None },
                SimpleTestRunner::map_status_code(&status)
            ),
            None => {
                child.kill().unwrap();
                (ExecutionMetrics { time: Some(self.timeout), memory_kilobytes: None }, Err(TimedOut))
            }
        }
    }
}

impl TestRunner for SimpleTestRunner {
    // pub fn test_to_file(&self, input_source: &TestInputSource, output_path: &Path) -> (ExecutionMetrics, Result<(), ExecutionError>) {
    //     let child = Command::new(&self.executable_path)
    //         .stdin(input_source.get_stdin())
    //         .stdout(output_path)
    //         .stderr(Stdio::null())
    //         .spawn().expect("Failed to spawn child");
    //     self.wait_for_child(child)
    // }

    fn test_to_string(&self, input_source: &TestInputSource) -> (ExecutionMetrics, Result<String, ExecutionError>) {
        let mut stdout = BufferedPipe::create().expect("Failed to create stdout pipe");

        let child = Command::new(&self.executable_path)
            .stdin(input_source.get_stdin())
            .stdout(stdout.get_stdio())
            .stderr(Stdio::null())
            .spawn().expect("Failed to spawn child");

        let (metrics, result) = self.wait_for_child(child);
        let output = stdout.join();
        (metrics, result.and_then(|_| output))
    }
}