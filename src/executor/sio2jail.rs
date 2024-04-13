use std::io::{read_to_string, Seek};
use std::os::fd::AsRawFd;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::Duration;
use colored::Colorize;
use command_fds::{CommandFdExt, FdMapping};
use directories::BaseDirs;
use memfile::MemFile;
use wait_timeout::ChildExt;
use which::which;
use crate::temp_files::{create_temp_file, make_cloned_stdio};
use crate::executor::TestExecutor;
use crate::formatted_error::FormattedError;
use crate::generic_utils::halt;
use crate::test_errors::{ExecutionError, ExecutionMetrics};
use crate::test_errors::ExecutionError::{MemoryLimitExceeded, RuntimeError, Sio2jailError, TimedOut};

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

    fn run_sio2jail(&self, input_stdio: Stdio, output_stdio: Stdio, executable_path: &Path) -> Result<Sio2jailOutput, ExecutionError> {
        let mut sio2jail_output = create_temp_file().unwrap();
        let mut stderr = create_temp_file().unwrap();

        let mut child = Command::new(&self.sio2jail_path)
            .args(["-f", "3", "-o", "oiaug", "--mount-namespace", "off", "--pid-namespace", "off", "--uts-namespace", "off", "--ipc-namespace", "off", "--net-namespace", "off", "--capability-drop", "off", "--user-namespace", "off", "-s", "-m", &self.memory_limit.to_string(), "--", executable_path.to_str().unwrap() ])
            .fd_mappings(vec![FdMapping {
                parent_fd: sio2jail_output.as_raw_fd(),
                child_fd: 3
            }]).expect("Failed to redirect file descriptor 3")
            .stdout(output_stdio)
            .stderr(make_cloned_stdio(&stderr))
            .stdin(input_stdio)
            .spawn().expect("Failed to spawn sio2jail");

        let status = child.wait_timeout(self.timeout).unwrap();
        let Some(status) = status else {
            child.kill().unwrap();
            return Err(TimedOut);
        };

        sio2jail_output.rewind().unwrap();
        stderr.rewind().unwrap();

        Ok(Sio2jailOutput {
            status,
            stderr: read_to_string(stderr).unwrap(),
            sio2jail_output: read_to_string(sio2jail_output).unwrap(),
        })
    }

    fn test(&self) -> Result<(), FormattedError> {
        let Ok(true_command_location) = which("true") else {
            return Err(FormattedError::from_str("The executable for the \"true\" command could not be found"));
        };

        let output = self.run_sio2jail(Stdio::null(), Stdio::null(), &true_command_location);
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                return Err(FormattedError::from_str(&format!("Sio2jail error: {}", error.to_string())));
            }
        };
        if output.stderr == "Exception occurred: System error occured: perf event open failed: Permission denied: error 13: Permission denied\n" {
            return Err(FormattedError::preformatted(format!(
                "{}\n{}",
                "You need to run the following command to use toster with sio2jail.\n\
                You may also put this option in your /etc/sysctl.conf.\n\
                This will make the setting persist across reboots.".red(),
                "sudo sysctl -w kernel.perf_event_paranoid=-1".white()
            )));
        }
        if !output.stderr.is_empty() {
            return Err(FormattedError::from_str(&format!("Sio2jail error: {}", output.stderr)));
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
        return Ok(executor)
    }
}

impl TestExecutor for Sio2jailExecutor {
    fn test_to_stdio(&self, input_stdio: Stdio, output_stdio: Stdio) -> (ExecutionMetrics, Result<(), ExecutionError>) {
        let output = match self.run_sio2jail(input_stdio, output_stdio, &self.executable_path) {
            Err(TimedOut) => {
                return (ExecutionMetrics { time: Some(self.timeout), memory_kibibytes: None }, Err(TimedOut));
            }
            Err(error) => {
                return (ExecutionMetrics::NONE, Err(error));
            }
            Ok(output) => output
        };

        if !output.stderr.is_empty() {
            return if output.stderr == "terminate called after throwing an instance of 'std::bad_alloc'\n  what():  std::bad_alloc\n" {
                (ExecutionMetrics { time: None, memory_kibibytes: Some(self.memory_limit) }, Err(MemoryLimitExceeded))
            } else {
                (ExecutionMetrics::NONE, Err(Sio2jailError(output.stderr)))
            }
        }

        let split: Vec<&str> = output.sio2jail_output.split_whitespace().collect();
        if split.len() < 6 {
            return (ExecutionMetrics::NONE, Err(Sio2jailError(format!("The sio2jail output is too short: {}", output.sio2jail_output))));
        }
        let sio2jail_status = split[0];
        let time = Duration::from_secs_f64(split[2].parse::<f64>().expect("Sio2jail returned an invalid runtime in the output") / 1000.0);
        let memory_kibibytes = split[4].parse::<u64>().expect("Sio2jail returned invalid memory usage in the output");
        let error_message = output.sio2jail_output.lines().nth(1);

        let metrics = ExecutionMetrics {
            time: Some(time),
            memory_kibibytes: Some(memory_kibibytes)
        };

        match output.status.code() {
            None => {
                #[cfg(all(unix))]
                if cfg!(unix) && output.status.signal().expect("Sio2jail returned an invalid status code") == 2 {
                    halt();
                }

                return (metrics, Err(RuntimeError(format!("- the process was terminated with the following error:\n{}", output.status.to_string()))))
            }
            Some(0) => {}
            Some(exit_code) => {
                return (metrics, Err(Sio2jailError(format!("Sio2jail returned an invalid status code: {}", exit_code))) );
            }
        }

        return (ExecutionMetrics { time: Some(time), memory_kibibytes: Some(memory_kibibytes) }, match sio2jail_status {
            "OK" => Ok(()),
            "RE" | "RV" => Err(RuntimeError(error_message.and_then(|message| Some(format!("- {}", message))).unwrap_or(String::new()))),
            "TLE" => Err(TimedOut),
            "MLE" => Err(MemoryLimitExceeded),
            "OLE" => Err(RuntimeError("- output limit exceeded".to_string())),
            _ => Err(Sio2jailError(format!("Sio2jail returned an invalid status in the output: {}", sio2jail_status)))
        });
    }
}
