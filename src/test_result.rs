use colored::Colorize;

pub struct ExecutionResult {
	pub(crate) memory_kilobytes: Option<i64>,
	pub(crate) time_seconds: f64
}

pub enum TestResult {
	Correct {
		test_name: String
	},
	Incorrect {
		test_name: String,
		diff: String
	},
	Error {
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
	RanOutOfMemory,
	RuntimeError(String),
	Sio2jailError(String)
}

impl TestResult {
	pub fn to_string(&self) -> String {
		let mut result: String = String::new();

		match self {
			TestResult::Correct { .. } => {}
			TestResult::Incorrect { test_name, diff } => {
				result.push_str(&format!("{}", format!("Test {}:\n", test_name).bold()));
				result.push_str(diff);
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
			TestResult::Correct { test_name } => test_name.clone(),
			TestResult::Incorrect { test_name, .. } => test_name.clone(),
			TestResult::Error { test_name, .. } => test_name.clone(),
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
			ExecutionError::RanOutOfMemory => "Memory limit exceeded".to_string(),
			ExecutionError::RuntimeError(error) => format!("Runtime error {}", error),
			ExecutionError::Sio2jailError(error) => format!("Sio2jail error: {}", error),
		};
	}
}
