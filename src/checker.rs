use std::io::{sink, Write};
use std::path::PathBuf;
use std::{io, thread};
use std::process::Stdio;
use std::time::Duration;
use colored::Colorize;
use crate::executor::simple::SimpleExecutor;
use crate::executor::TestExecutor;
use crate::prepare_input::TestInputSource;
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

    pub(crate) fn check(&self, input_source: &TestInputSource, output: &str) -> Result<(), TestError> {
        let (mut reader, mut writer) = os_pipe::pipe().expect("Failed to create checker input pipe");
        let output = thread::scope(|scope| {
            let handle = scope.spawn(|| {
                io::copy(&mut input_source.read(), &mut writer).unwrap();
                writer.write("\n".as_bytes()).unwrap();
                writer.write_all(output.as_bytes()).unwrap();
                drop(writer);
            });
            let (_, output) = self.executor.test_to_string(
                Stdio::from(reader.try_clone().unwrap())
            );
            io::copy(&mut reader, &mut sink()).expect("Failed to flush checker input");
            handle.join().expect("Checker input writer panicked");
            output
        });
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                return Err(CheckerError { error });
            }
        };
        return Self::parse_checker_output(&output);
    }
}