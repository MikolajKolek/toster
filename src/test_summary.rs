use std::cmp::Ordering;
use colored::Colorize;
use crate::test_result::{ExecutionError, TestResult};
use crate::test_result::TestResult::*;

pub(crate) struct TestSummary {
    pub(crate) total: usize,
    pub(crate) success: usize,
    pub(crate) incorrect: usize,
    pub(crate) timed_out: usize,
    pub(crate) invalid_output: usize,
    pub(crate) memory_limit_exceeded: usize,
    pub(crate) runtime_error: usize,
    pub(crate) sio2jail_error: usize,
    pub(crate) checker_error: usize,
    pub(crate) no_output_file: usize,

    incorrect_test_results: Vec<TestResult>,
}

impl TestSummary {
    pub(crate) fn new() -> Self {
        TestSummary {
            total: 0,
            incorrect: 0,
            timed_out: 0,
            invalid_output: 0,
            memory_limit_exceeded: 0,
            runtime_error: 0,
            sio2jail_error: 0,
            checker_error: 0,
            no_output_file: 0,
            success: 0,

            incorrect_test_results: vec![],
        }
    }

    pub(crate) fn increment_success(&mut self) {
        self.total += 1;
        self.success += 1;
    }

    pub(crate) fn add_test_result(&mut self, result: TestResult) {
        self.total += 1;
        match result {
            Correct { .. } => { self.success += 1 }
            Incorrect { .. } => { self.incorrect += 1 }
            ProgramError { error: ExecutionError::TimedOut, .. } => { self.timed_out += 1 }
            ProgramError { error: ExecutionError::InvalidOutput, .. } => { self.invalid_output += 1 }
            ProgramError { error: ExecutionError::MemoryLimitExceeded, .. } => { self.memory_limit_exceeded += 1 }
            ProgramError { error: ExecutionError::RuntimeError(_), .. } => { self.runtime_error += 1 }
            ProgramError { error: ExecutionError::Sio2jailError(_), .. } => { self.sio2jail_error += 1 }
            ProgramError { error: ExecutionError::IncorrectCheckerFormat(_), .. } => { self.checker_error += 1 }
            CheckerError { .. } => { self.checker_error += 1 }
            NoOutputFile { .. } => { self.no_output_file += 1 }
        }

        if !result.is_correct() {
            self.incorrect_test_results.push(result);
        }
    }

    pub(crate) fn format_error_counts(&self) -> String {
        let mut res = [
            (self.incorrect, if self.incorrect > 1 { "wrong answers" } else { "wrong answer" }, ),
            (self.timed_out, "timed out"),
            (self.invalid_output, if self.invalid_output > 1 { "invalid outputs" } else { "invalid output" }),
            (self.memory_limit_exceeded, "out of memory"),
            (self.runtime_error, if self.runtime_error > 1 { "runtime errors" } else { "runtime error" }),
            (self.no_output_file, if self.no_output_file > 1 { "without output files" } else { "without output file" }),
            (self.sio2jail_error, if self.sio2jail_error > 1 { "sio2jail errors" } else { "sio2jail error" })
        ]
            .into_iter()
            .filter(|(count, _)| count > &0)
            .map(|(count, label)| format!("{} {}", count.to_string().red(), label.to_string().red()))
            .collect::<Vec<String>>()
            .join(", ");

        if self.checker_error > 0 {
            res += &format!(
                "{}{}{}",
                if res.is_empty() { "" } else { ", " },
                self.checker_error.to_string().blue(),
                (if self.checker_error > 1 { " checker errors" } else { " checker error" }).blue()
            );
        }

        res
    }

    pub(crate) fn get_incorrect_results(&mut self) -> &Vec<TestResult> {
        self.incorrect_test_results.sort_unstable_by(|a, b| -> Ordering {
            human_sort::compare(&a.test_name(), &b.test_name())
        });
        &self.incorrect_test_results
    }
}