use std::fs::File;
use std::io::{read_to_string, Seek, Write};
use std::path::PathBuf;
use std::io;
use std::process::Stdio;
use std::time::Duration;
use colored::Colorize;
use crate::executor::simple::SimpleExecutor;
use crate::executor::test_to_temp;
use crate::prepare_input::TestInputSource;
use crate::temp_files::create_temp_file;
use crate::test_errors::TestError;
use crate::test_errors::ExecutionError::IncorrectCheckerFormat;
use crate::test_errors::TestError::CheckerError;

pub(crate) struct Checker {
    executor: SimpleExecutor
}

impl Checker {
    pub(crate) fn new(checker_executable: PathBuf, timeout: Duration) -> Self {
        Checker {
            executor: SimpleExecutor {
                executable_path: checker_executable,
                timeout,
            }
        }
    }

    fn parse_checker_output(output: &str) -> Result<(), TestError> {
        match output.chars().nth(0) {
            None => Err(CheckerError { error: IncorrectCheckerFormat("the checker returned an empty file".to_string()) }),
            Some('C') => Ok(()),
            Some('N') => {
                let checker_error = if output.len() > 1 { output.split_at(2).1.to_string() } else { String::new() };
                let error_message = format!("Incorrect output{}{}", if checker_error.trim().is_empty() { "" } else { ": " }, checker_error.trim()).red();
                Err(TestError::Incorrect {
                    error: error_message.to_string(),
                })
            }
            Some(_) => Err(CheckerError { error: IncorrectCheckerFormat("the first character of the checker's output wasn't C or I".to_string()) })
        }
    }

    /// Creates a new temporary file for the checker input and writes the program input to it.
    /// The cursor is left at the end (not rewound).
    ///
    /// The program output should be appended to this file before calling check() on it,
    /// which can be done by passing the file as stdin to the tested program.
    pub(crate) fn prepare_checker_input(input_source: &TestInputSource) -> File {
        let mut input_memfile = create_temp_file().unwrap();
        io::copy(&mut input_source.read(), &mut input_memfile).unwrap();
        input_memfile.write_all("\n".as_bytes()).unwrap();
        input_memfile
    }

    /// Run checker on input file created using `prepare_checker_input()`.
    /// The program output should be appended to that file.
    /// `check()` will rewind `checker_input` before running checker.
    pub(crate) fn check(&self, mut checker_input: File) -> Result<(), TestError> {
        checker_input.rewind().unwrap();

        let (_, result) = test_to_temp(&self.executor, Stdio::from(checker_input));
        let output = match result {
            Ok(output) => output,
            Err(error) => {
                return Err(CheckerError { error });
            }
        };
        let output = read_to_string(output).expect("Failed to read checker output");
        Self::parse_checker_output(&output)
    }
}