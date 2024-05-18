use std::time::Duration;
use colored::Colorize;

pub struct ExecutionMetrics {
    pub(crate) memory_kibibytes: Option<u64>,
    pub(crate) time: Option<Duration>,
}

impl ExecutionMetrics {
    // Currently only the sio2jail executor uses this constant,
    // which is not compiled on Windows builds
    #[allow(dead_code)]
    pub const NONE: ExecutionMetrics = ExecutionMetrics { memory_kibibytes: None, time: None };
}

pub enum TestError {
    Incorrect {
        error: String
    },
    ProgramError {
        error: ExecutionError
    },
    CheckerError {
        error: ExecutionError
    },
    NoOutputFile,
    Cancelled,
}

#[allow(unused)]
#[derive(Debug)]
pub enum ExecutionError {
    TimedOut,
    MemoryLimitExceeded,
    RuntimeError(String),
    Sio2jailError(String),
    PipeError,
    OutputNotUtf8,
    IncorrectCheckerFormat(String),
}

impl TestError {
    pub fn to_string(&self, test_name: &str) -> String {
        let mut result: String = String::new();

        match self {
            TestError::Incorrect { error } => {
                result.push_str(&format!("{}", format!("Test {}:\n", test_name).bold()));
                result.push_str(error);
            }
            TestError::ProgramError { error } => {
                result.push_str(&format!("{}", format!("Test {}:\n", test_name).bold()));
                result.push_str(&format!("{}", error.to_string().red()));
            }
            TestError::CheckerError { error } => {
                result.push_str(&format!("{}", format!("Test {} encountered a checker error:\n", test_name).bold()));
                result.push_str(&format!("{}", error.to_string().blue()));
            }
            TestError::NoOutputFile => {
                result.push_str(&format!("{}", format!("Test {}:\n", test_name).bold()));
                result.push_str(&format!("{}", "Output file does not exist".red()));
            }
            TestError::Cancelled => {
                result.push_str(&format!("{}", format!("Test {}:\n", test_name).bold()));
                result.push_str(&format!("{}", "Cancelled".yellow()));
            }
        }

        result
    }
}

impl ExecutionError {
    pub fn to_string(&self) -> String {
        match self {
            ExecutionError::TimedOut => "Timed out".to_string(),
            ExecutionError::MemoryLimitExceeded => "Memory limit exceeded".to_string(),
            ExecutionError::RuntimeError(error) => format!("Runtime error {}", error),
            ExecutionError::Sio2jailError(error) => format!("Sio2jail error: {}", error),
            ExecutionError::IncorrectCheckerFormat(error) => format!("The checker output didn't follow the Toster checker format - {}", error),
            ExecutionError::PipeError => "Failed to read program output".to_string(),
            ExecutionError::OutputNotUtf8 => "The output contained invalid characters".to_string(),
        }
    }
}
