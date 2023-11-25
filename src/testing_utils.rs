use std::cmp::max;
use std::fs;
use std::path::Path;
use comfy_table::{Attribute, Cell, Color, Table};
use comfy_table::ContentArrangement::Dynamic;
use terminal_size::{Height, Width};
use crate::test_errors::TestError;
use crate::test_errors::TestError::{Incorrect, NoOutputFile};

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
