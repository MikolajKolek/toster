use std::cmp::max;
use colored::Colorize;
use comfy_table::ContentArrangement::Dynamic;
use comfy_table::{Attribute, Cell, Color, Table};
use terminal_size::{Height, Width};

pub enum TestResult {
	Correct {
		test_name: String
	},
	Incorrect {
		test_name: String,
		correct_answer: String,
		incorrect_answer: String
	},
	Error {
		test_name: String,
		error: ExecutionError
	},
	NoOutputFile {
		test_name: String
	}
}

pub enum ExecutionError {
	TimedOut,
	NonZeroReturn(i32),
	Terminated(String)
}

impl TestResult {
	pub fn to_string(&self) -> String {
		let mut result: String = String::new();

		match self {
			TestResult::Correct {test_name} => {
				result.push_str(&format!("{}", format!("Test {}:\n", test_name).bold()));
				result.push_str(&format!("{}", "Timed out".red()));
			}
			TestResult::Incorrect {test_name, correct_answer, incorrect_answer} => {
				result.push_str(&format!("{}", format!("Test {}:\n", test_name).bold()));

				let split_correct = correct_answer.split("\n").collect::<Vec<_>>();
				let split_incorrect = incorrect_answer.split("\n").collect::<Vec<_>>();

				let (Width(w), Height(_)) = terminal_size::terminal_size().unwrap_or((Width(40), Height(0)));

				let mut table = Table::new();
				table.set_content_arrangement(Dynamic).set_width(w).set_header(vec![
					Cell::new("Line").add_attribute(Attribute::Bold),
					Cell::new("Output file").add_attribute(Attribute::Bold).fg(Color::Green),
					Cell::new("Your program's output").add_attribute(Attribute::Bold).fg(Color::Red)
				]);

				let mut row_count = 0;
				for i in 0..max(split_correct.len(), split_incorrect.len()) {
					let file_segment = if split_correct.len() > i { split_correct[i] } else { "" };
					let out_segment = if split_incorrect.len() > i { split_incorrect[i] } else { "" };

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

				result.push_str(&table.to_string());
			}
			TestResult::NoOutputFile { test_name } => {
				result.push_str(&format!("{}", format!("Test {}:\n", test_name).bold()));
				result.push_str(&format!("{}", "Output file does not exist".red()));
			}
			TestResult::Error { test_name, error } => {
				result.push_str(&format!("{}", format!("Test {}:\n", test_name).bold()));
				result.push_str(&format!("{}", error.to_string().red()));
			}
		}

		return result;
	}

	pub fn test_name(&self) -> String {
		return match self {
			TestResult::Correct { test_name } => {test_name.clone()}
			TestResult::Incorrect { test_name, .. } => {test_name.clone()}
			TestResult::Error { test_name, .. } => {test_name.clone()}
			TestResult::NoOutputFile { test_name } => {test_name.clone()}
		};
	}
}

impl ExecutionError {
	pub fn to_string(&self) -> String {
		return match self {
			ExecutionError::TimedOut => {
				"Timed out".to_string()
			}
			ExecutionError::NonZeroReturn(code) => {
				format!("Runtime error - the program returned a non-zero return code: {}", code)
			}
			ExecutionError::Terminated(message) => {
				format!("Runtime error - the process was terminated with the following error:\n{}", message)
			}
		}
	}
}