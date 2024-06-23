use std::fmt;
use std::fmt::{Display, Formatter};
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
                result.push_str(&format!("{}", format!("Test {test_name}:\n").bold()));
                result.push_str(error);
            }
            TestError::ProgramError { error } => {
                result.push_str(&format!("{}", format!("Test {test_name}:\n").bold()));
                result.push_str(&format!("{}", error.to_string().red()));
            }
            TestError::CheckerError { error } => {
                result.push_str(&format!("{}", format!("Test {test_name} encountered a checker error:\n").bold()));
                result.push_str(&format!("{}", error.to_string().blue()));
            }
            TestError::NoOutputFile => {
                result.push_str(&format!("{}", format!("Test {test_name}:\n").bold()));
                result.push_str(&format!("{}", "Output file does not exist".red()));
            }
            TestError::Cancelled => {
                result.push_str(&format!("{}", format!("Test {test_name}:\n").bold()));
                result.push_str(&format!("{}", "Cancelled".yellow()));
            }
        }

        result
    }
}

impl Display for ExecutionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ExecutionError::TimedOut => write!(f, "Timed out"),
            ExecutionError::MemoryLimitExceeded => write!(f, "Memory limit exceeded"),
            ExecutionError::RuntimeError(error) => write!(f, "Runtime error {error}"),
            ExecutionError::Sio2jailError(error) => write!(f, "Sio2jail error: {error}"),
            ExecutionError::IncorrectCheckerFormat(error) => write!(f, "The checker output didn't follow the Toster checker format - {error}"),
            ExecutionError::PipeError => write!(f, "Failed to read program output"),
            ExecutionError::OutputNotUtf8 => write!(f, "The output contained invalid characters"),
        }
    }
}
