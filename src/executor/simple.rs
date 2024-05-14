use std::fs::File;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use crate::executor::common::Watchdog;
use crate::test_errors::{ExecutionError, ExecutionMetrics};
use crate::executor::TestExecutor;
use crate::flag::Flag;
use crate::test_errors::ExecutionError::{RuntimeError};

use crate::owned_child::{CommandExt, ExitStatus};
use crate::temp_files::make_cloned_stdio;

pub(crate) struct SimpleExecutor {
    executable_path: PathBuf,
    watchdog: Watchdog,
}

impl SimpleExecutor {
    pub(crate) fn init(timeout: Duration, executable_path: PathBuf, kill_flag: &'static Flag) -> Self {
        SimpleExecutor {
            executable_path,
            watchdog: Watchdog::start(timeout, kill_flag),
        }
    }

    fn map_exit_status(status: &ExitStatus) -> Result<(), ExecutionError> {
        match status {
            ExitStatus::ExitCode(0) => Ok(()),
            ExitStatus::ExitCode(exit_code) => {
                Err(RuntimeError(format!("- the program returned a non-zero return code: {}", exit_code)))
            },
            ExitStatus::Signalled(signal) => {
                // TODO: Format signal
                Err(RuntimeError(format!("- the program terminated due to a signal {}", signal)))
            }
        }
    }
}

impl TestExecutor for SimpleExecutor {
    fn test_to_file(&self, input_file: &File, output_file: &File) -> (ExecutionMetrics, Result<(), ExecutionError>) {
        let child = Command::new(&self.executable_path)
            .stdin(make_cloned_stdio(input_file))
            .stdout(make_cloned_stdio(output_file))
            .stderr(Stdio::null())
            .spawn_owned().expect("Failed to spawn child");

        self.watchdog.add_handle(child.get_handle());

        let start_time = Instant::now();
        // TODO: Handle timeout status
        let status = child.wait().unwrap();
        (
            ExecutionMetrics { time: Some(start_time.elapsed()), memory_kibibytes: None },
            SimpleExecutor::map_exit_status(&status),
        )
    }
}
