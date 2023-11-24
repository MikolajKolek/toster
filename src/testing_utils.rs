use std::cmp::max;
use std::fs;
use std::fs::File;
use std::io::ErrorKind::NotFound;
use std::io::{Stdout, Write};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::os::fd::AsRawFd;
#[cfg(all(unix))]
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{ChildStdout, Command, Stdio};
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
use crate::{Correct, ProgramError, Incorrect, TestResult};
use crate::prepare_input::{Test, TestInputSource};
use crate::run::BasicTestRunner;
use crate::test_result::{ExecutionError, ExecutionMetrics};
use crate::test_result::ExecutionError::{InvalidOutput, RuntimeError, TimedOut, IncorrectCheckerFormat};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use crate::test_result::ExecutionError::{MemoryLimitExceeded, Sio2jailError};
use crate::test_result::TestResult::CheckerError;
use crate::TestResult::NoOutputFile;

static SIO2JAIL_PATH: OnceLock<PathBuf> = OnceLock::new();
static TEMPFILE_POOL: Lazy<ArrayQueue<PathBuf>> = Lazy::new(|| { ArrayQueue::new(num_cpus::get() * 10) });

pub fn fill_tempfile_pool(tempdir: &TempDir) {
	for i in 0..(num_cpus::get() * 10) {
		let file_path = tempdir.path().join(format!("tempfile-{}", i));
		TEMPFILE_POOL.push(file_path).expect("Couldn't push into tempfile pool");
	}
}

pub fn init_sio2jail() -> bool {
	let Some(binding) = BaseDirs::new() else {
		println!("{}", "No valid home directory path could be retrieved from the operating system. Sio2jail could not be found".red());
		return false;
	};
	let Some(executable_dir) = binding.executable_dir() else {
		println!("{}", "Couldn't locate the user's executable directory. Sio2jail could not be found".red());
		return false;
	};

	let result = executable_dir.join("sio2jail");
	if !result.exists() {
		println!("{}{}", "Sio2jail could not be found at ".red(), result.display().to_string().red());
		return false;
	}

	SIO2JAIL_PATH.get_or_init(|| result);
	return true;
}

pub fn compile_cpp(
	source_code_path: &Path,
	tempdir: &TempDir,
	compile_timeout: u64,
	compile_command: &str,
) -> Result<(PathBuf, f64), String> {
	let executable_file_base = source_code_path.file_stem().expect("The provided filename is invalid!");
	let executable_path = tempdir.path().join(format!("{}.o", executable_file_base.to_str().expect("The provided filename is invalid!")));

	let cmd = compile_command
		.replace("<IN>", source_code_path.to_str().expect("The provided filename is invalid!"))
		.replace("<OUT>", &executable_path.to_str().expect("The provided filename is invalid"));
	let mut split_cmd = cmd.split(" ");

	let compilation_result_path = tempdir.path().join(format!("{}.out", executable_file_base.to_str().expect("The provided filename is invalid!")));
	let compilation_result_file = File::create(&compilation_result_path).expect("Failed to create temporary file!");
	let time_before_compilation = Instant::now();
	let child = Command::new(&split_cmd.nth(0).expect("The compile command is invalid!"))
		.args(split_cmd)
		.stderr(compilation_result_file)
		.spawn();

	let mut child = match child {
		Ok(child) => child,
		Err(error) if error.kind() == NotFound => { return Err("The compiler was not found!".to_string()) }
		Err(error) => { return Err(error.to_string()) }
	};

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

	Ok((executable_path, compilation_time))
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub fn generate_output_sio2jail(
	executable_path: &Path,
	input_source: &TestInputSource,
	output_file: File,
	timeout: &u64,
	memory_limit: &u64,
	sio2jail_output_file_path: &Path,
	sio2jail_output_file: File,
	error_file_path: &Path,
	error_file: File
) -> (ExecutionMetrics, Result<(), ExecutionError>) {
	let mut command = Command::new(SIO2JAIL_PATH.get().expect("Sio2jail was not properly initialized!"))
		.args(["-f", "3", "-o", "oiaug", "--mount-namespace", "off", "--pid-namespace", "off", "--uts-namespace", "off", "--ipc-namespace", "off", "--net-namespace", "off", "--capability-drop", "off", "--user-namespace", "off", "-s", "-m", &memory_limit.to_string(), "--", executable_path ])
		.fd_mappings(vec![FdMapping {
			parent_fd: sio2jail_output_file.as_raw_fd(),
			child_fd: 3
		}]).expect("Failed to redirect file descriptor 3!")
		.stdout(output_file)
		.stderr(error_file);

	match input_source {
		TestInputSource::File(path) => {
			command.stdin(path);
		}
	}

	let mut child = command.spawn().expect("Failed to run file!");

	let status = child.wait_timeout(Duration::from_secs(*timeout)).unwrap();
	let Some(status) = status else {
		child.kill().unwrap();
		return (ExecutionMetrics { time_seconds: *timeout as f64, memory_kilobytes: None }, Err(TimedOut));
	};

	let error_output = fs::read_to_string(error_file_path).expect("Couldn't read sio2jail error output");
	if !error_output.is_empty() {
		return if error_output == "terminate called after throwing an instance of 'std::bad_alloc'\n  what():  std::bad_alloc\n" {
			(ExecutionMetrics { time_seconds: 0f64, memory_kilobytes: Some(*memory_limit as i64) }, Err(MemoryLimitExceeded))
		} else {
			(ExecutionMetrics { time_seconds: 0f64, memory_kilobytes: None }, Err(Sio2jailError(error_output)))
		}
	}

	let sio2jail_output = fs::read_to_string(sio2jail_output_file_path).expect("Couldn't read temporary sio2jail file");
	let split: Vec<&str> = sio2jail_output.split_whitespace().collect();
	if split.len() < 6 {
		return (ExecutionMetrics { time_seconds: 0f64, memory_kilobytes: None }, Err(Sio2jailError(format!("The sio2jail output is too short: {}", sio2jail_output))));
	}
	let sio2jail_status = split[0];
	let time_seconds = split[2].parse::<f64>().expect("Sio2jail returned an invalid runtime in the output") / 1000.0;
	let memory_kilobytes = split[4].parse::<i64>().expect("Sio2jail returned invalid memory usage in the output");
	let error_message = sio2jail_output.lines().nth(1);

	match status.code() {
		None => {
			#[cfg(all(unix))]
			if cfg!(unix) && status.signal().expect("Sio2jail returned an invalid status code!") == 2 {
				thread::sleep(Duration::from_secs(u64::MAX));
			}

			return (ExecutionMetrics { time_seconds, memory_kilobytes: Some(memory_kilobytes) }, Err(RuntimeError(format!    ("- the process was terminated with the following error:\n{}", status.to_string()))))
		}
		Some(0) => {}
		Some(exit_code) => {
			return (ExecutionMetrics { time_seconds, memory_kilobytes: Some(memory_kilobytes) }, Err(Sio2jailError(format!("Sio2jail returned an invalid status code: {}", exit_code))) );
		}
	}

	return (ExecutionMetrics { time_seconds, memory_kilobytes: Some(memory_kilobytes) }, match sio2jail_status {
		"OK" => Ok(()),
		"RE" | "RV" => Err(RuntimeError(error_message.and_then(|message| Some(format!("- {}", message))).unwrap_or(String::new()))),
		"TLE" => Err(TimedOut),
		"MLE" => Err(MemoryLimitExceeded),
		"OLE" => Err(RuntimeError(format!("- output limit exceeded"))),
		_ => Err(Sio2jailError(format!("Sio2jail returned an invalid status in the output: {}", sio2jail_status)))
	});
}

// pub fn checker_verify(
// 	test_name: &str,
// 	checker_path: &Path,
// 	input_source: &TestInputSource,
// 	program_output: &str,
// 	checker_input_file_path: &Path,
// 	mut checker_input_file: File,
// 	checker_output_file_path: &Path,
// 	checker_output_file: File,
// 	timeout: &u64
// ) -> TestResult {
// 	checker_input_file.write_all(format!("{}\n{}", program_input, program_output).as_bytes()).expect("Failed to write to checker input file!");
// 	drop(checker_input_file);
// 	let checker_input_file_readable = File::open(checker_input_file_path).expect("Couldn't open checker input file!");
//
// 	let mut child = Command::new(checker_path)
// 		.stdout(checker_output_file)
// 		.stdin(checker_input_file_readable)
// 		.spawn()
// 		.expect("Failed to run checker!");
//
// 	return match child.wait_timeout(Duration::from_secs(*timeout)).unwrap() {
// 		Some(status) => {
// 			let Some(exit_code) = status.code() else {
// 				#[cfg(all(unix))]
// 				if cfg!(unix) && status.signal().expect("The checker returned an invalid status code!") == 2 {
// 					thread::sleep(Duration::from_secs(u64::MAX));
// 				}
//
// 				return CheckerError { test_name: test_name.to_string(), error: RuntimeError(format!("- the process was terminated with the following error:\n{}", status.to_string())) }
// 			};
// 			if exit_code != 0 {
// 				return CheckerError { test_name: test_name.to_string(), error: RuntimeError(format!("- the checker returned a non-zero return code: {}", status.code().unwrap())) }
// 			}
//
// 			let checker_output = fs::read_to_string(checker_output_file_path).expect("Couldn't read checker output file!");
//
// 			match checker_output.chars().nth(0) {
// 				None => CheckerError { test_name: test_name.to_string(), error: IncorrectCheckerFormat("the checker returned an empty file".to_string()) },
// 				Some('C') => Correct { test_name: test_name.to_string() },
// 				Some('N') => {
// 					let checker_error = if checker_output.len() > 1 { checker_output.split_at(2).1.to_string() } else { String::new() };
// 					let error_message = format!("Incorrect output{}{}", if checker_error.trim().is_empty() { "" } else { ": " }, checker_error.trim()).red();
//
// 					Incorrect { test_name: test_name.to_string(), error: error_message.to_string() }
// 				}
// 				Some(_) => CheckerError { test_name: test_name.to_string(), error: IncorrectCheckerFormat("the first character of the checker's output wasn't C or I".to_string()) }
// 			}
// 		}
// 		None => {
// 			child.kill().unwrap();
// 			CheckerError { test_name: test_name.to_string(), error: TimedOut }
// 		}
// 	};
// }

// pub fn run_test(
// 	executable_path: &Path,
// 	checker_path: Option<&Path>,
// 	input_source: &TestInputSource,
// 	output_dir: &Path,
// 	test_name: &str,
// 	out_extension: &str,
// 	timeout: &Duration,
// 	_use_sio2jail: bool,
// 	_memory_limit: u64,
// ) -> (TestResult, ExecutionMetrics) {
// 	let test_runner = BasicTestRunner {
// 		timeout: timeout.clone(),
// 		executable_path: executable_path.to_path_buf(),
// 	};
//
// 	#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
// 	let (execution_metrics, execution_result) = if _use_sio2jail {
// 		todo!();
// 		// let sio2jail_output_file_path = TEMPFILE_POOL.pop().expect("Couldn't acquire tempfile!");
// 		// let sio2jail_output_file = File::create(&sio2jail_output_file_path).expect("Failed to create temporary file!");
// 		// let error_file_path = TEMPFILE_POOL.pop().expect("Couldn't acquire tempfile!");
// 		// let error_file = File::create(&error_file_path).expect("Failed to create temporary file!");
// 		//
// 		// let result = generate_output_sio2jail(executable_path, input_file, test_output_file, timeout, &_memory_limit, &sio2jail_output_file_path, sio2jail_output_file, &error_file_path, error_file);
// 		//
// 		// TEMPFILE_POOL.push(sio2jail_output_file_path).expect("Couldn't push into tempfile pool");
// 		// TEMPFILE_POOL.push(error_file_path).expect("Couldn't push into tempfile pool");
// 		//
// 		// result
// 	} else {
// 		test_runner.test_to_vec(input_source)
// 	};
// 	#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
// 	let (execution_result, execution_error) = generate_output_direct(executable_path, input_file, test_output_file, timeout);
//
// 	let output = match execution_result {
// 		Err(error) => {
// 			return (ProgramError { test_name: test_name.to_string(), error }, execution_metrics);
// 		}
// 		Ok(output) => output
// 	};
//
// 	let Ok(output) = String::from_utf8(output) else {
// 		return (ProgramError { test_name: test_name.to_string(), error: InvalidOutput }, execution_metrics);
// 	};
//
// 	return if let Some(checker_path) = checker_path {
// 		let checker_input_file_path = TEMPFILE_POOL.pop().expect("Couldn't acquire tempfile!");
// 		let checker_input_file = File::create(&checker_input_file_path).expect("Failed to create temporary file!");
// 		let checker_output_file_path = TEMPFILE_POOL.pop().expect("Couldn't acquire tempfile!");
// 		let checker_output_file = File::create(&checker_output_file_path).expect("Failed to create temporary file!");
//
// 		let result = (
// 			checker_verify(test_name,
// 				&checker_path,
// 				input_source,
// 				&output,
// 				&checker_input_file_path,
// 				checker_input_file,
// 				&checker_output_file_path,
// 				checker_output_file,
// 				timeout
// 			),
// 			execution_result
// 		);
//
// 		TEMPFILE_POOL.push(checker_input_file_path).expect("Couldn't push into tempfile pool");
// 		TEMPFILE_POOL.push(checker_output_file_path).expect("Couldn't push into tempfile pool");
//
// 		result
// 	} else {
// 		let correct_output_file_path = output_dir.join(format!("{}{}", &test_name, &out_extension));
// 		if !correct_output_file_path.is_file() {
// 			return (NoOutputFile { test_name: test_name.to_string() }, ExecutionMetrics { time_seconds: 0f64, memory_kilobytes: None });
// 		}
// 		let correct_output = fs::read_to_string(correct_output_file_path).expect("Failed to read output file!");
//
// 		let is_correct = split_trim_end(&test_output) == split_trim_end(&correct_output);
// 		if is_correct {
// 			(Correct { test_name: test_name.to_string() }, execution_result)
// 		} else {
// 			(Incorrect { test_name: test_name.to_string(), error: generate_diff(correct_output, test_output) }, execution_result)
// 		}
// 	}
// }

fn split_trim_end(to_split: &str) -> Vec<String> {
	let mut res: Vec<String> = Vec::new();

	let mut current_string = String::new();
	for ch in to_split.chars() {
		if ch == '\n' {
			res.push(current_string.trim_end().to_string());
			current_string = String::new();
		}
		else {
			current_string.push(ch);
		}
	}

	if !current_string.is_empty() {
		res.push(current_string.trim_end().to_string());
	}

	while res.last().is_some_and(|last| last.trim().is_empty()) {
		res.pop();
	}

	return res;
}

fn generate_diff(correct_answer: String, incorrect_answer: String) -> String {
	let correct_split = split_trim_end(&correct_answer);
	let incorrect_split = split_trim_end(&incorrect_answer);

	let (Width(w), Height(_)) = terminal_size::terminal_size().unwrap_or((Width(40), Height(0)));
	let mut table = Table::new();
	table.set_content_arrangement(Dynamic).set_width(w).set_header(vec![
		Cell::new("Line").add_attribute(Attribute::Bold),
		Cell::new("Output file").add_attribute(Attribute::Bold).fg(Color::Green),
		Cell::new("Your program's output").add_attribute(Attribute::Bold).fg(Color::Red)
	]);

	let mut row_count = 0;
	for i in 0..max(correct_split.len(), incorrect_split.len()) {
		let file_segment = if correct_split.len() > i { correct_split[i].clone() } else { "".to_string() };
		let out_segment = if incorrect_split.len() > i { incorrect_split[i].clone() } else { "".to_string() };

		if file_segment != out_segment {
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

	return table.to_string().replace("\r", "");
}
