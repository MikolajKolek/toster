use std::cmp::max;
use std::fs;
use std::fs::File;
use std::io::ErrorKind::NotFound;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use comfy_table::{Attribute, Cell, Color, Table};
use comfy_table::ContentArrangement::Dynamic;
use tempfile::TempDir;
use terminal_size::{Height, Width};
use wait_timeout::ChildExt;
use crate::test_errors::TestError;
use crate::test_errors::TestError::{Incorrect, NoOutputFile};

pub fn compile_cpp(
	source_code_path: &Path,
	tempdir: &TempDir,
	compile_timeout: Duration,
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

	match child.wait_timeout(compile_timeout).unwrap() {
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

pub(crate) fn compare_output(expected_output_path: &Path, actual_output: &str) -> Result<(), TestError> {
	if !expected_output_path.is_file() {
		return Err(NoOutputFile);
	}
	let expected_output = fs::read_to_string(expected_output_path).expect("Failed to read output file!");

	let expected_output = split_trim_end(&expected_output);
	let actual_output = split_trim_end(&actual_output);

	if actual_output != expected_output {
		return Err(Incorrect { error: generate_diff(&expected_output, &actual_output) });
	}
	Ok(())
}

fn split_trim_end(to_split: &str) -> Vec<&str> {
	let mut res = to_split
		.split('\n')
		.map(|line| line.trim_end())
		.collect::<Vec<&str>>();

	while res.last().is_some_and(|last| last.trim().is_empty()) {
		res.pop();
	}

	return res;
}

fn generate_diff(expected_split: &[&str], actual_split: &[&str]) -> String {
	let (Width(w), Height(_)) = terminal_size::terminal_size().unwrap_or((Width(40), Height(0)));
	let mut table = Table::new();
	table.set_content_arrangement(Dynamic).set_width(w).set_header(vec![
		Cell::new("Line").add_attribute(Attribute::Bold),
		Cell::new("Output file").add_attribute(Attribute::Bold).fg(Color::Green),
		Cell::new("Your program's output").add_attribute(Attribute::Bold).fg(Color::Red)
	]);

	let mut row_count = 0;
	for i in 0..max(expected_split.len(), actual_split.len()) {
		let expected_line = expected_split.get(i).unwrap_or(&"");
		let actual_line = actual_split.get(i).unwrap_or(&"");

		if expected_line != actual_line {
			table.add_row(vec![
				Cell::new(i + 1),
				Cell::new(expected_line).fg(Color::Green),
				Cell::new(actual_line).fg(Color::Red)
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
