use colored::Colorize;

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
			TestResult::Incorrect {test_name, diff} => {
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