use std::cmp::max;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command};
use std::time::{Duration, Instant};
use tempfile::{TempDir};
use wait_timeout::ChildExt;
use colored::Colorize;
use comfy_table::ContentArrangement::Dynamic;
use comfy_table::{Attribute, Cell, Color, Table};
use terminal_size::{Height, Width};
use crate::{Correct, Error, Incorrect, TestResult};
use crate::test_result::ExecutionError;
use crate::test_result::ExecutionError::{NonZeroReturn, Terminated, TimedOut};
use crate::TestResult::{NoOutputFile};

pub fn compile_cpp(source_code_file: PathBuf, tempdir: &TempDir, compile_timeout: u64) -> Result<String, String> {
	let source_code_folder = source_code_file.parent().expect("The source code is in an invalid folder!");
	let executable_file_base = source_code_folder.join(source_code_file.file_stem().expect("The provided filename is invalid!"));
	let executable_file = tempdir.path().join(format!("{}.o", executable_file_base.to_str().expect("The provided filename is invalid!"))).to_str().expect("The provided filename is invalid!").to_string();
	let compilation_result_path = tempdir.path().join(format!("{}.out", executable_file_base.to_str().expect("The provided filename is invalid!")));
	let compilation_result_file = File::create(&compilation_result_path).expect("Failed to create temporary file!");

	let time_before_compilation = Instant::now();
	let mut child = Command::new("g++")
		.args(["-std=c++17", "-O3", "-static", source_code_file.to_str().expect("The provided filename is invalid!"), "-o", &executable_file])
		.stderr(compilation_result_file)
		.spawn().expect("g++ failed to start");
	match child.wait_timeout(Duration::from_secs(compile_timeout)).unwrap() {
		Some(status) => {
			if status.code().expect("G++ returned an invalid status code") != 0 {
				let compilation_result = fs::read_to_string(&compilation_result_path).expect("Failed to read G++ output");
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

pub fn generate_output(executable_path: &String, input_file: File, output_file: File, timeout: &u64) -> Result<f64, (ExecutionError, f64)> {
	let time_before_run = Instant::now();
	let mut child = Command::new(executable_path)
		.stdout(output_file)
		.stdin(input_file)
		.spawn().expect("Failed to run compiled file!");

	return match child.wait_timeout(Duration::from_secs(*timeout)).unwrap() {
		Some(status) => {
			if status.code().is_none() {
				return Err((Terminated(status.to_string()), time_before_run.elapsed().as_secs_f64()));
			}
			if status.code().unwrap() != 0 {
				return Err((NonZeroReturn(status.code().unwrap()), time_before_run.elapsed().as_secs_f64()));
			}

			Ok(time_before_run.elapsed().as_secs_f64())
		}
		None => {
			child.kill().unwrap();
			Err((TimedOut, *timeout as f64))
		}
	};
}

pub fn run_test(executable_path: &String,
            input_file_path: &Path,
            output_dir: &String,
            test_name: &String,
            out_extension: &String,
            tempdir: &TempDir,
            timeout: &u64) -> (TestResult, f64) {
	let input_file = File::open(input_file_path).expect("Failed to open input file!");

	let correct_output_file_path = format!("{}/{}{}", &output_dir, &test_name, &out_extension);
	if !Path::new(&correct_output_file_path).is_file() {
		return (NoOutputFile {test_name: test_name.clone()}, 0 as f64);
	}
	let test_output_file_path = tempdir.path().join(format!("{}.out", test_name));
	let test_output_file = File::create(&test_output_file_path).expect("Failed to create temporary file!");

	let test_time_result = generate_output(executable_path, input_file, test_output_file, timeout);
	if test_time_result.is_err() {
		let result = test_time_result.unwrap_err();
		return (Error {test_name: test_name.clone(), error: result.0}, result.1);
	}
	let test_time = test_time_result.unwrap_or_default();

	let test_output: String = fs::read_to_string(&test_output_file_path).expect("Failed to read temporary file!");
	let correct_output = fs::read_to_string(Path::new(&correct_output_file_path)).expect("Failed to read output file!");
	return if test_output.split_whitespace().collect::<Vec<&str>>() != correct_output.split_whitespace().collect::<Vec<&str>>() {
		(Incorrect { test_name: test_name.clone(), diff: generate_diff(correct_output, test_output) }, test_time)
	} else {
		(Correct { test_name: test_name.clone() }, test_time)
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