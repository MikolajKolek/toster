use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;
use command_fds::{CommandFdExt, FdMapping};
use directories::BaseDirs;
use wait_timeout::ChildExt;
use crate::pipes::BufferedPipe;
use crate::prepare_input::TestInputSource;
use crate::run::TestRunner;
use crate::test_errors::{ExecutionError, ExecutionMetrics};
use crate::test_errors::ExecutionError::{MemoryLimitExceeded, RuntimeError, Sio2jailError, TimedOut};

pub(crate) struct Sio2jailRunner {
    timeout: Duration,
    executable_path: PathBuf,
    sio2jail_path: PathBuf,
    memory_limit: u64,
}

fn get_sio2jail_path() -> Result<PathBuf, String> {
    let Some(binding) = BaseDirs::new() else {
        return Err("No valid home directory path could be retrieved from the operating system. Sio2jail could not be found".to_string());
    };
    let Some(executable_dir) = binding.executable_dir() else {
        return Err("Couldn't locate the user's executable directory. Sio2jail could not be found".to_string());
    };

    let result = executable_dir.join("sio2jail");
    if !result.exists() {
        return Err(format!("Sio2jail could not be found at {}", result.display()));
    }
    Ok(result)
}

impl Sio2jailRunner {
    pub(crate) fn init(timeout: Duration, executable_path: PathBuf, memory_limit: u64) -> Result<Sio2jailRunner, String> {
        Ok(Sio2jailRunner {
            timeout,
            memory_limit,
            executable_path,
            sio2jail_path: get_sio2jail_path()?,
        })
    }
}

impl TestRunner for Sio2jailRunner {
    fn test_to_string(&self, input_source: &TestInputSource) -> (ExecutionMetrics, Result<String, ExecutionError>) {
        let sio2jail_output = BufferedPipe::create().expect("Failed to create sio2jail output pipe");
        let mut stdout = BufferedPipe::create().expect("Failed to create stdout pipe");
        let mut stderr = BufferedPipe::create().expect("Failed to create stderr pipe");

        let mut child = Command::new(&self.sio2jail_path)
            .args(["-f", "3", "-o", "oiaug", "--mount-namespace", "off", "--pid-namespace", "off", "--uts-namespace", "off", "--ipc-namespace", "off", "--net-namespace", "off", "--capability-drop", "off", "--user-namespace", "off", "-s", "-m", &self.memory_limit.to_string(), "--", self.executable_path.to_str().unwrap() ])
            .fd_mappings(vec![FdMapping {
                parent_fd: sio2jail_output.get_raw_fd(),
                child_fd: 3
            }]).expect("Failed to redirect file descriptor 3!")
            .stdout(stdout.get_stdio())
            .stderr(stderr.get_stdio())
            .stdin(input_source.get_stdin())
            .spawn().expect("Failed to run file!");

        let status = child.wait_timeout(self.timeout).unwrap();
        let Some(status) = status else {
            child.kill().unwrap();
            return (ExecutionMetrics { time: Some(self.timeout), memory_kilobytes: None }, Err(TimedOut));
        };

        let stderr = match stderr.join() {
            Err(error) => return (ExecutionMetrics::NONE, Err(error)),
            Ok(output) => output,
        };
        let sio2jail_output = match sio2jail_output.join() {
            Err(error) => return (ExecutionMetrics::NONE, Err(error)),
            Ok(output) => output,
        };
        let stdout = match stdout.join() {
            Err(error) => return (ExecutionMetrics::NONE, Err(error)),
            Ok(output) => output,
        };

        if !stderr.is_empty() {
            return if stderr == "terminate called after throwing an instance of 'std::bad_alloc'\n  what():  std::bad_alloc\n" {
                (ExecutionMetrics { time: None, memory_kilobytes: Some(self.memory_limit) }, Err(MemoryLimitExceeded))
            } else {
                (ExecutionMetrics::NONE, Err(Sio2jailError(stderr)))
            }
        }

        let split: Vec<&str> = sio2jail_output.split_whitespace().collect();
        if split.len() < 6 {
            return (ExecutionMetrics::NONE, Err(Sio2jailError(format!("The sio2jail output is too short: {}", sio2jail_output))));
        }
        let sio2jail_status = split[0];
        let time = Duration::from_secs_f64(split[2].parse::<f64>().expect("Sio2jail returned an invalid runtime in the output") / 1000.0);
        let memory_kilobytes = split[4].parse::<u64>().expect("Sio2jail returned invalid memory usage in the output");
        let error_message = sio2jail_output.lines().nth(1);

        let metrics = ExecutionMetrics {
            time: Some(time),
            memory_kilobytes: Some(memory_kilobytes)
        };

        match status.code() {
            None => {
                #[cfg(all(unix))]
                if cfg!(unix) && status.signal().expect("Sio2jail returned an invalid status code!") == 2 {
                    thread::sleep(Duration::from_secs(u64::MAX));
                }

                return (metrics, Err(RuntimeError(format!    ("- the process was terminated with the following error:\n{}", status.to_string()))))
            }
            Some(0) => {}
            Some(exit_code) => {
                return (metrics, Err(Sio2jailError(format!("Sio2jail returned an invalid status code: {}", exit_code))) );
            }
        }

        return (ExecutionMetrics { time: Some(time), memory_kilobytes: Some(memory_kilobytes) }, match sio2jail_status {
            "OK" => Ok(stdout),
            "RE" | "RV" => Err(RuntimeError(error_message.and_then(|message| Some(format!("- {}", message))).unwrap_or(String::new()))),
            "TLE" => Err(TimedOut),
            "MLE" => Err(MemoryLimitExceeded),
            "OLE" => Err(RuntimeError(format!("- output limit exceeded"))),
            _ => Err(Sio2jailError(format!("Sio2jail returned an invalid status in the output: {}", sio2jail_status)))
        });
    }
}
