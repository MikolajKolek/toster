use std::cmp::Ordering;
use colored::Color::{Blue, Green, Red, Yellow};
use colored::{Color, Colorize};
use crate::test_result::{ExecutionError, TestResult};
use crate::test_result::TestResult::*;

pub(crate) struct TestSummary {
    generate_mode: bool,

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

struct CountPart<'a> {
    display_empty: bool,
    count: usize,
    singular: &'a str,
    plural: &'a str,
    color: Color,
}

impl<'a> CountPart<'a> {
    fn new(count: usize, text: &'a str) -> Self {
        CountPart {
            display_empty: false,
            count,
            singular: text,
            plural: text,
            color: Red
        }
    }

    fn with_plural(mut self, text: &'a str) -> Self {
        self.plural = text;
        self
    }

    fn display_empty(mut self) -> Self {
        self.display_empty = true;
        self
    }

    fn with_color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    fn get_text(&self) -> &str {
        if self.count == 1 { self.singular }
        else { self.plural }
    }
}

impl TestSummary {
    pub(crate) fn new(generate_mode: bool) -> Self {
        TestSummary {
            generate_mode,

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

    pub(crate) fn format_counts(&self, not_finished: Option<usize>) -> String {
        [
            CountPart::new(self.success, if self.generate_mode { "successful" } else { "correct" }).display_empty().with_color(Green),
            CountPart::new(self.incorrect, "wrong answer").with_plural("wrong answers"),
            CountPart::new(self.timed_out, "timed out"),
            CountPart::new(self.invalid_output, "invalid output").with_plural("invalid outputs"),
            CountPart::new(self.memory_limit_exceeded, "out of memory"),
            CountPart::new(self.runtime_error, "runtime error").with_plural("runtime errors"),
            CountPart::new(self.no_output_file, "without output file"),
            CountPart::new(self.sio2jail_error, "sio2jail error").with_plural("sio2jail errors"),
            CountPart::new(self.checker_error, "checker error").with_plural("checker errors").with_color(Blue),
            CountPart::new(not_finished.unwrap_or(0), "not finished").with_color(Yellow),
        ]
            .into_iter()
            .filter(|part| part.display_empty || part.count > 0)
            .map(|part| {
                format!("{} {}", part.count, part.get_text()).color(part.color).to_string()
            })
            .collect::<Vec<String>>()
            .join(", ")
    }

    pub(crate) fn get_incorrect_results(&mut self) -> &Vec<TestResult> {
        self.incorrect_test_results.sort_unstable_by(|a, b| -> Ordering {
            human_sort::compare(&a.test_name(), &b.test_name())
        });
        &self.incorrect_test_results
    }
}