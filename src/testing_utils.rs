use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use tempfile::{TempDir};
use wait_timeout::ChildExt;
use colored::Colorize;
use crate::{Correct, Error, Incorrect, TestResult};

pub fn compile_cpp(source_code_file: PathBuf, tempdir: &TempDir) -> Result<String, String> {
	let source_code_folder = source_code_file.parent().expect("The source code is in an invalid folder!");
	let executable_file_base = source_code_folder.join(source_code_file.file_stem().expect("The provided filename is invalid!"));
	let executable_file = format!("{}.o", executable_file_base.to_str().expect("The provided filename is invalid!"));
	let compilation_result_path = tempdir.path().join(format!("{}.out", executable_file_base.to_str().expect("The provided filename is invalid!")));
	let compilation_result_file = File::create(&compilation_result_path).expect("Failed to create temporary file!");

	let time_before_compilation = Instant::now();
	if Command::new("g++")
		.args(["-std=c++17", "-O3", "-static", source_code_file.to_str().expect("The provided filename is invalid!"), "-o", &executable_file])
		.stderr(compilation_result_file)
		.spawn().expect("g++ failed to start").wait_timeout(Duration::from_secs(10)).is_err() {
		return Err("Compilation took too long".to_string());
	}
	let compilation_time = time_before_compilation.elapsed().as_secs_f64();

	let compilation_result = fs::read_to_string(&compilation_result_path).expect("Failed to read G++ output");
	return if !compilation_result.is_empty() {
		Err(compilation_result)
	}
	else {
		println!("{}", format!("Compilation completed in {:.2}s", compilation_time).green());
		Ok(executable_file)
	}
}

pub fn generate_output(executable_path: &String, input_file: File, output_file: File, timeout: &u64) -> f64 {
	let time_before_run = Instant::now();
	let mut child = Command::new(format!("./{}", &executable_path))
		.stdout(output_file)
		.stdin(input_file)
		.spawn().expect("Failed to run compiled file!");

	return if child.wait_timeout(Duration::from_secs(*timeout)).unwrap() == None {
		*timeout as f64
	}
	else {
		time_before_run.elapsed().as_secs_f64()
	}
}

pub fn run_test(executable_path: &String,
            input_dir: &String,
            output_dir: &String,
            test_name: &String,
            tempdir: &TempDir,
            timeout: &u64) -> (TestResult, f64) {
	let input_file = File::open(format!("{}/{}.in", &input_dir, &test_name)).expect("Failed to open input file!");

	let correct_output_file_path = format!("{}/{}.out", &output_dir, &test_name);
	if !Path::new(&correct_output_file_path).is_file() {
		return (Error {test_name: test_name.clone(), error_message: "Output file does not exist".to_string()}, 0 as f64);
	}
	let test_output_file_path = tempdir.path().join(format!("{}.out", test_name));
	let test_output_file = File::create(&test_output_file_path).expect("Failed to create temporary file!");

	let test_time = generate_output(executable_path, input_file, test_output_file, timeout);
	if test_time == *timeout as f64 {
		return (Error {test_name: test_name.clone(), error_message: "Timed out".to_string()}, *timeout as f64);
	}

	let test_output: String = fs::read_to_string(&test_output_file_path).expect("Failed to read temporary file!");
	let correct_output = fs::read_to_string(Path::new(&correct_output_file_path)).expect("Failed to read output file!");
	return if test_output.split_whitespace().collect::<Vec<&str>>() != correct_output.split_whitespace().collect::<Vec<&str>>() {
		(Incorrect { test_name: test_name.clone(), correct_answer: correct_output, incorrect_answer: test_output }, test_time)
	} else {
		(Correct { test_name: test_name.clone() }, test_time)
	}
}