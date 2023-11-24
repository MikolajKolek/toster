use std::time::Duration;
use colored::Colorize;

pub struct ExecutionMetrics {
	pub(crate) memory_kilobytes: Option<i64>,
	pub(crate) time: Duration,
}

pub enum TestResult {
	Correct {
		test_name: String
	},
	Incorrect {
		test_name: String,
		error: String
	},
	ProgramError {
		test_name: String,
		error: ExecutionError
	},
	CheckerError {
		test_name: String,
		error: ExecutionError
	},
	NoOutputFile {
		test_name: String
	}
}

#[allow(unused)]
pub enum ExecutionError {
	TimedOut,
	InvalidOutput,
	MemoryLimitExceeded,
	RuntimeError(String),
	Sio2jailError(String),
	IncorrectCheckerFormat(String)
}

impl TestResult {
	pub fn to_string(&self) -> String {
		let mut result: String = String::new();

		match self {
			TestResult::Correct { .. } => {}
			TestResult::Incorrect { test_name, error } => {
				result.push_str(&format!("{}", format!("Test {}:\n", test_name).bold()));
				result.push_str(error);
			}
			TestResult::ProgramError { test_name, error } => {
				result.push_str(&format!("{}", format!("Test {}:\n", test_name).bold()));
				result.push_str(&format!("{}", error.to_string().red()));
			}
			TestResult::CheckerError { test_name, error } => {
				result.push_str(&format!("{}", format!("Test {} encountered a checker error:\n", test_name).bold()));
				result.push_str(&format!("{}", error.to_string().blue()));
			}
			TestResult::NoOutputFile { test_name } => {
				result.push_str(&format!("{}", format!("Test {}:\n", test_name).bold()));
				result.push_str(&format!("{}", "Output file does not exist".red()));
			}
		}

		return result;
	}

	pub fn test_name(&self) -> String {
		return match self {
			TestResult::Correct { test_name } => test_name.clone(),
			TestResult::Incorrect { test_name, .. } => test_name.clone(),
			TestResult::ProgramError { test_name, .. } => test_name.clone(),
			TestResult::CheckerError { test_name, .. } => test_name.clone(),
			TestResult::NoOutputFile { test_name } => test_name.clone()
		};
	}

	pub fn is_correct(&self) -> bool {
		match self {
			TestResult::Correct { .. } => true,
			_ => false,
		}
	}
}

impl ExecutionError {
	pub fn to_string(&self) -> String {
		return match self {
			ExecutionError::TimedOut => "Timed out".to_string(),
			ExecutionError::InvalidOutput => "The output contained invalid characters".to_string(),
			ExecutionError::MemoryLimitExceeded => "Memory limit exceeded".to_string(),
			ExecutionError::RuntimeError(error) => format!("Runtime error {}", error),
			ExecutionError::Sio2jailError(error) => format!("Sio2jail error: {}", error),
			ExecutionError::IncorrectCheckerFormat(error) => format!("The checker output didn't follow the Toster checker format - {}", error)
		};
	}
}
