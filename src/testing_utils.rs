use std::cmp::max;
use std::fs;
use std::fs::File;
use std::io::ErrorKind::NotFound;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::os::fd::AsRawFd;
#[cfg(all(unix))]
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
#[cfg(all(unix))]
use std::thread;
use std::time::{Duration, Instant};
use colored::Colorize;
use comfy_table::{Attribute, Cell, Color, Table};
use comfy_table::ContentArrangement::Dynamic;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use command_fds::{CommandFdExt, FdMapping};
use crossbeam_queue::ArrayQueue;
use directories::BaseDirs;
use once_cell::sync::Lazy;
use tempfile::TempDir;
use terminal_size::{Height, Width};
use wait_timeout::ChildExt;
use crate::{Correct, Error, Incorrect, TestResult};
use crate::test_result::{ExecutionError, ExecutionResult};
use crate::test_result::ExecutionError::{RuntimeError, TimedOut};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use crate::test_result::ExecutionError::{MemoryLimitExceeded, Sio2jailError};
use crate::TestResult::NoOutputFile;

static SIO2JAIL_PATH: OnceLock<String> = OnceLock::new();
static TEMPFILE_POOL: Lazy<ArrayQueue<PathBuf>> = Lazy::new(|| { ArrayQueue::new(num_cpus::get() * 10) });

pub fn fill_tempfile_pool(tempdir: &TempDir) {
	for i in 0..(num_cpus::get() * 10) {
		let file_path = tempdir.path().join(format!("tempfile-{}", i));
		TEMPFILE_POOL.push(file_path).expect("Couldn't push into tempfile pool");
	}
}

pub fn init_sio2jail() -> bool {
	let base_dirs = BaseDirs::new();
	if base_dirs.is_none() {
		println!("{}", "No valid home directory path could be retrieved from the operating system. Sio2jail could not be found".red());
		return false;
	}
	let binding = base_dirs.unwrap();
	let executable_dir = binding.executable_dir();
	if executable_dir.is_none() {
		println!("{}", "Couldn't locate the user's executable directory. Sio2jail could not be found".red());
		return false;
	}
	let executable_dir_str = executable_dir.unwrap().to_str();
	if executable_dir_str.is_none() {
		println!("{}", "The user's executable directory is invalid. Sio2jail could not be found".red());
		return false;
	}

	let result = format!("{}/sio2jail", executable_dir_str.unwrap());
	if !Path::new(&result).exists() {
		println!("{}{}", "Sio2jail could not be found at ".red(), result.red());
		return false;
	}

	SIO2JAIL_PATH.get_or_init(|| { return result; });
	return true;
}

pub fn compile_cpp(
	source_code_file: PathBuf,
	tempdir: &TempDir,
	compile_timeout: u64,
	compile_command: &String,
) -> Result<String, String> {
	let executable_file_base = source_code_file.file_stem().expect("The provided filename is invalid!");
	let executable_file = tempdir.path().join(format!("{}.o", executable_file_base.to_str().expect("The provided filename is invalid!"))).to_str().expect("The provided filename is invalid!").to_string();

	let cmd = compile_command
		.replace("<IN>", source_code_file.to_str().expect("The provided filename is invalid!"))
		.replace("<OUT>", &executable_file);
	let mut split_cmd = cmd.split(" ");

	let compilation_result_path = tempdir.path().join(format!("{}.out", executable_file_base.to_str().expect("The provided filename is invalid!")));
	let compilation_result_file = File::create(&compilation_result_path).expect("Failed to create temporary file!");
	let time_before_compilation = Instant::now();
	let command = Command::new(&split_cmd.nth(0).expect("The compile command is invalid!"))
		.args(split_cmd.collect::<Vec<&str>>())
		.stderr(compilation_result_file)
		.spawn();

	if command.as_ref().is_err() {
		return if matches!(command.as_ref().unwrap_err().kind(), NotFound) {
			Err("The compiler was not found!".to_string())
		} else {
			Err(command.unwrap_err().to_string())
		}
	}

	let mut child = command.unwrap();
	match child.wait_timeout(Duration::from_secs(compile_timeout)).unwrap() {
		Some(status) => {
			if status.code().expect("The compiler returned an invalid status code") != 0 {
				let compilation_result = fs::read_to_string(&compilation_result_path).expect("Failed to read compiler output");
				return Err(compilation_result);
			}
		}
		None => {
			child.kill().unwrap();
			return Err("Compilation timed out".to_string());
		}
	}
	let compilation_time = time_before_compilation.elapsed().as_secs_f64();

	println!("{}", format!("Compilation completed in {:.2}s", compilation_time).green());
	Ok(executable_file)
}

pub fn generate_output_default(
	executable_path: &String,
	input_file: File,
	output_file: File,
	timeout: &u64,
) -> (ExecutionResult, Result<(), ExecutionError>) {
	let time_before_run = Instant::now();
	let mut child = Command::new(executable_path)
		.stdout(output_file)
		.stdin(input_file)
		.spawn()
		.expect("Failed to run file!");

	return match child.wait_timeout(Duration::from_secs(*timeout)).unwrap() {
		Some(status) => {
			if status.code().is_none() {
				#[cfg(all(unix))]
				if cfg!(unix) && status.signal().expect("The program returned an invalid status code!") == 2 {
					thread::sleep(Duration::from_secs(u64::MAX));
				}

				return (ExecutionResult { time_seconds: time_before_run.elapsed().as_secs_f64(), memory_kilobytes: None }, Err(RuntimeError(format!("- the process was terminated with the following error:\n{}", status.to_string()))))
			}
			if status.code().unwrap() != 0 {
				return (ExecutionResult { time_seconds: time_before_run.elapsed().as_secs_f64(), memory_kilobytes: None }, Err(RuntimeError(format!("- the program returned a non-zero return code: {}", status.code().unwrap()))))
			}

			(ExecutionResult { time_seconds: time_before_run.elapsed().as_secs_f64(), memory_kilobytes: None }, Ok(()))
		}
		None => {
			child.kill().unwrap();
			(ExecutionResult { time_seconds: *timeout as f64, memory_kilobytes: None }, Err(TimedOut))
		}
	};
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn generate_output_sio2jail(
	executable_path: &String,
	input_file: File,
	output_file: File,
	timeout: &u64,
	memory_limit: &u64,
	sio2jail_output_file_path: &PathBuf,
	sio2jail_output_file: File,
	error_file_path: &PathBuf,
	error_file: File
) -> (ExecutionResult, Result<(), ExecutionError>) {
	let mut child = Command::new(SIO2JAIL_PATH.get().expect("Sio2jail was not properly initialized!"))
		.args(["-f", "3", "-o", "oiaug", "--mount-namespace", "off", "--pid-namespace", "off", "--uts-namespace", "off", "--ipc-namespace", "off", "--net-namespace", "off", "--capability-drop", "off", "--user-namespace", "off", "-s", "-m", &memory_limit.to_string(), "--", executable_path ])
		.fd_mappings(vec![FdMapping {
			parent_fd: sio2jail_output_file.as_raw_fd(),
			child_fd: 3
		}]).expect("Failed to redirect file descriptor 3!")
		.stdout(output_file)
		.stdin(input_file)
		.stderr(error_file)
		.spawn()
		.expect("Failed to run file!");

	let command_result = child.wait_timeout(Duration::from_secs(*timeout)).unwrap();
	if command_result.is_none() {
		child.kill().unwrap();
		return (ExecutionResult { time_seconds: *timeout as f64, memory_kilobytes: None }, Err(TimedOut));
	}

	let error_output = fs::read_to_string(error_file_path).expect("Couldn't read sio2jail error output");
	if !error_output.is_empty() {
		return if error_output == "terminate called after throwing an instance of 'std::bad_alloc'\n  what():  std::bad_alloc\n" {
			(ExecutionResult { time_seconds: 0 as f64, memory_kilobytes: Some(*memory_limit as i64) }, Err(MemoryLimitExceeded))
		} else {
			(ExecutionResult { time_seconds: 0 as f64, memory_kilobytes: None }, Err(Sio2jailError(error_output)))
		}
	}

	let sio2jail_output = fs::read_to_string(sio2jail_output_file_path).expect("Couldn't read temporary sio2jail file");
	let split: Vec<&str> = sio2jail_output.split_whitespace().collect();
	if split.len() < 6 {
		return (ExecutionResult { time_seconds: 0 as f64, memory_kilobytes: None }, Err(Sio2jailError(format!("The sio2jail output is too short: {}", sio2jail_output))));
	}
	let sio2jail_status = split[0];
	let time_seconds = split[2].parse::<f64>().expect("Sio2jail returned an invalid runtime in the output") / 1000.0;
	let memory_kilobytes = split[4].parse::<i64>().expect("Sio2jail returned invalid memory usage in the output");
	let error_message = sio2jail_output.lines().nth(1);

	let status = command_result.unwrap();
	if status.code().is_none() {
		#[cfg(all(unix))]
		if cfg!(unix) && status.signal().expect("Sio2jail returned an invalid status code!") == 2 {
			thread::sleep(Duration::from_secs(u64::MAX));
		}

		return (ExecutionResult { time_seconds, memory_kilobytes: Some(memory_kilobytes) }, Err(RuntimeError(format!    ("- the process was terminated with the following error:\n{}", status.to_string()))))
	}
	if status.code().unwrap() != 0 {
		return (ExecutionResult { time_seconds, memory_kilobytes: Some(memory_kilobytes) }, Err(Sio2jailError(format!("Sio2jail returned an invalid status code: {}", status.code().unwrap()))) );
	}

	return (ExecutionResult { time_seconds, memory_kilobytes: Some(memory_kilobytes) }, match sio2jail_status {
		"OK" => Ok(()),
		"RE" | "RV" => Err(RuntimeError(if error_message.is_none() { String::new() } else { format!("- {}", error_message.unwrap()) })),
		"TLE" => Err(TimedOut),
		"MLE" => Err(MemoryLimitExceeded),
		"OLE" => Err(RuntimeError(format!("- output limit exceeded"))),
		_ => Err(Sio2jailError(format!("Sio2jail returned an invalid status in the output: {}", sio2jail_status)))
	});
}

pub fn run_test(
	executable_path: &String,
	input_file_path: &Path,
	output_dir: &String,
	test_name: &String,
	out_extension: &String,
	timeout: &u64,
	_use_sio2jail: bool,
	_memory_limit: u64,
) -> (TestResult, ExecutionResult) {
	let input_file = File::open(input_file_path).expect("Failed to open input file!");

	let correct_output_file_path = format!("{}/{}{}", &output_dir, &test_name, &out_extension);
	if !Path::new(&correct_output_file_path).is_file() {
		return (NoOutputFile {test_name: test_name.clone()}, ExecutionResult { time_seconds: 0 as f64, memory_kilobytes: None });
	}
	let test_output_file_path = TEMPFILE_POOL.pop().expect("Couldn't acquire tempfile!");
	let test_output_file = File::create(&test_output_file_path).expect("Failed to create temporary file!");

	#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
	let (execution_result, execution_error) = if _use_sio2jail {
		let sio2jail_output_file_path = TEMPFILE_POOL.pop().expect("Couldn't acquire tempfile!");
		let sio2jail_output_file = File::create(&sio2jail_output_file_path).expect("Failed to create temporary file!");
		let error_file_path = TEMPFILE_POOL.pop().expect("Couldn't acquire tempfile!");
		let error_file = File::create(&error_file_path).expect("Failed to create temporary file!");

		let result = generate_output_sio2jail(executable_path, input_file, test_output_file, timeout, &_memory_limit, &sio2jail_output_file_path, sio2jail_output_file, &error_file_path, error_file);

		TEMPFILE_POOL.push(sio2jail_output_file_path).expect("Couldn't push into tempfile pool");
		TEMPFILE_POOL.push(error_file_path).expect("Couldn't push into tempfile pool");

		result
	} else {
		generate_output_default(executable_path, input_file, test_output_file, timeout)
	};
	#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
	let (execution_result, execution_error) = generate_output_default(executable_path, input_file, test_output_file, timeout);

	if execution_error.is_err() {
		TEMPFILE_POOL.push(test_output_file_path).expect("Couldn't push into tempfile pool");

		let result = execution_error.unwrap_err();
		return (Error { test_name: test_name.clone(), error: result }, execution_result);
	}

	let test_output: String = fs::read_to_string(&test_output_file_path).expect("Failed to read temporary file!");
	let correct_output = fs::read_to_string(Path::new(&correct_output_file_path)).expect("Failed to read output file!");
	let is_correct = test_output.split_whitespace().collect::<Vec<&str>>() == correct_output.split_whitespace().collect::<Vec<&str>>();
	TEMPFILE_POOL.push(test_output_file_path).expect("Couldn't push into tempfile pool");
	return if is_correct {
		(Correct { test_name: test_name.clone() }, execution_result)
	} else {
		(Incorrect { test_name: test_name.clone(), diff: generate_diff(correct_output, test_output) }, execution_result)
	}
}

fn generate_diff(correct_answer: String, incorrect_answer: String) -> String {
	let correct_split = correct_answer.split("\n").collect::<Vec<_>>();
	let incorrect_split = incorrect_answer.split("\n").collect::<Vec<_>>();

	let (Width(w), Height(_)) = terminal_size::terminal_size().unwrap_or((Width(40), Height(0)));
	let mut table = Table::new();
	table.set_content_arrangement(Dynamic).set_width(w).set_header(vec![
		Cell::new("Line").add_attribute(Attribute::Bold),
		Cell::new("Output file").add_attribute(Attribute::Bold).fg(Color::Green),
		Cell::new("Your program's output").add_attribute(Attribute::Bold).fg(Color::Red)
	]);

	let mut row_count = 0;
	for i in 0..max(correct_split.len(), incorrect_split.len()) {
		let file_segment = if correct_split.len() > i { correct_split[i] } else { "" };
		let out_segment = if incorrect_split.len() > i { incorrect_split[i] } else { "" };

		if file_segment.split_whitespace().collect::<Vec<&str>>() != out_segment.split_whitespace().collect::<Vec<&str>>() {
			table.add_row(vec![
				Cell::new(i + 1),
				Cell::new(file_segment).fg(Color::Green),
				Cell::new(out_segment).fg(Color::Red)
			]);

			row_count += 1;
		}

		if row_count >= 99 {
			table.add_row(vec![
				Cell::new("..."),
				Cell::new("..."),
				Cell::new("...")
			]);

			break;
		}
	}

	return table.to_string();
}