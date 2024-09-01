    use crate::executor::TestExecutor;
use crate::formatted_error::FormattedError;
use crate::test_errors::ExecutionError::{MemoryLimitExceeded, RuntimeError, Sio2jailError, TimedOut};
use crate::test_errors::{ExecutionError, ExecutionMetrics};
use colored::Colorize;
use directories::BaseDirs;
use perfjail::process::Feature::PERF;
use perfjail::process::{ExitStatus, Perfjail};
use std::fs::File;
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::time::Duration;

pub(crate) struct Sio2jailExecutor {
    timeout: Duration,
    executable_path: PathBuf,
    sio2jail_path: PathBuf,
    memory_limit: u64,
}

struct Sio2jailOutput {
    status: ExitStatus,
    stderr: String,
    sio2jail_output: String,
}

impl Sio2jailExecutor {
    fn get_sio2jail_path() -> Result<PathBuf, FormattedError> {
        let Some(binding) = BaseDirs::new() else {
            return Err(FormattedError::from_str(
                "No valid home directory path could be retrieved from the operating system. Sio2jail could not be found"
            ));
        };
        let Some(executable_dir) = binding.executable_dir() else {
            return Err(FormattedError::from_str(
                "Couldn't locate the user's executable directory. Sio2jail could not be found"
            ));
        };

        let result = executable_dir.join("sio2jail");
        if !result.exists() {
            return Err(FormattedError::from_str(
                &format!("Sio2jail could not be found at {}", result.display())
            ));
        }
        Ok(result)
    }

    fn test(&self) -> Result<(), FormattedError> {
        if !perfjail::setup::test_perf().unwrap_or(false) {
            return Err(FormattedError::preformatted(format!(
                "{}\n{}",
                "You need to run the following command to use toster with sio2jail.\n\
                You may also put this option in your /etc/sysctl.conf.\n\
                This will make the setting persist across reboots.".red(),
                "sudo sysctl -w kernel.perf_event_paranoid=-1".white()
            )));
        }

        Ok(())
    }

    pub(crate) fn init_and_test(timeout: Duration, executable_path: PathBuf, memory_limit: u64) -> Result<Sio2jailExecutor, FormattedError> {
        let executor = Sio2jailExecutor {
            timeout,
            memory_limit,
            executable_path,
            sio2jail_path: Self::get_sio2jail_path()?,
        };
        executor.test()?;
        Ok(executor)
    }
}

impl TestExecutor for Sio2jailExecutor {
    fn test_to_file(&self, input_file: &File, output_file: &File) -> (ExecutionMetrics, Result<(), ExecutionError>) {
        let jail_result = Perfjail::new(&self.executable_path)
            .stdin(input_file.as_fd())
            .stdout(output_file.as_fd())
            .features(PERF)
            .spawn()
            .expect("Failed to spawn perfjail child process")
            .run();
        
        if let Ok(jail_result) = jail_result {
            (ExecutionMetrics { time: jail_result.measured_time, memory_kibibytes: None }, match jail_result.exit_status {
                ExitStatus::OK => Ok(()),
                ExitStatus::RE(_) | ExitStatus::RV(_) => Err(RuntimeError(format!("- {}", jail_result.exit_status.get_exit_status_comment()))),
                ExitStatus::TLE(_) => Err(TimedOut),
                ExitStatus::MLE(_) => Err(MemoryLimitExceeded),
                ExitStatus::OLE(_) => Err(RuntimeError("- output limit exceeded".to_string())),
            })
        } else {
            (ExecutionMetrics { time: None, memory_kibibytes: None }, Err(Sio2jailError(String::new())))
        }
    }
}
