use std::cmp::Ordering;
use std::time::{Duration, Instant};
use colored::Color::{Blue, Green, Red, Yellow};
use colored::{Color, Colorize};
use crate::generic_utils::OptionExt;
use crate::test_errors::{ExecutionError, ExecutionMetrics, TestError};
use crate::test_errors::TestError::*;

pub(crate) struct TestSummary {
    pub(crate) generate_mode: bool,
    pub(crate) start_time: Instant,

    pub(crate) total: usize,
    pub(crate) processed: usize,
    pub(crate) success: usize,
    pub(crate) incorrect: usize,
    pub(crate) timed_out: usize,
    pub(crate) invalid_output: usize,
    pub(crate) memory_limit_exceeded: usize,
    pub(crate) runtime_error: usize,
    pub(crate) sio2jail_error: usize,
    pub(crate) checker_error: usize,
    pub(crate) no_output_file: usize,

    test_errors: Vec<(String, TestError)>,

    pub(crate) slowest_test: Option<(Duration, String)>,
    pub(crate) most_memory_used: Option<(u64, String)>,
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
    pub(crate) fn new(generate_mode: bool, total_count: usize) -> Self {
        TestSummary {
            generate_mode,
            start_time: Instant::now(),

            total: total_count,
            processed: 0,
            incorrect: 0,
            timed_out: 0,
            invalid_output: 0,
            memory_limit_exceeded: 0,
            runtime_error: 0,
            sio2jail_error: 0,
            checker_error: 0,
            no_output_file: 0,
            success: 0,

            test_errors: vec![],

            slowest_test: None,
            most_memory_used: None,
        }
    }

    pub(crate) fn add_success(&mut self, metrics: &ExecutionMetrics, test_name: &str) {
        self.processed += 1;
        self.success += 1;
        self.add_metrics(metrics, test_name);
    }

    pub(crate) fn add_test_error(&mut self, error: TestError, test_name: String) {
        self.processed += 1;
        match &error {
            Incorrect { .. } => { self.incorrect += 1 }
            ProgramError { error: ExecutionError::TimedOut, .. } => { self.timed_out += 1 }
            ProgramError { error: ExecutionError::MemoryLimitExceeded, .. } => { self.memory_limit_exceeded += 1 }
            ProgramError { error: ExecutionError::RuntimeError(_), .. } => { self.runtime_error += 1 }
            ProgramError { error: ExecutionError::Sio2jailError(_), .. } => { self.sio2jail_error += 1 }
            ProgramError { error: ExecutionError::IncorrectCheckerFormat(_), .. } => { self.checker_error += 1 }
            ProgramError { error: ExecutionError::PipeError } => { self.invalid_output += 1 }
            ProgramError { error: ExecutionError::OutputNotUtf8 } => { self.invalid_output += 1 }
            CheckerError { .. } => { self.checker_error += 1 }
            NoOutputFile { .. } => { self.no_output_file += 1 }
        }
        self.test_errors.push((test_name, error));
    }

    fn add_metrics(&mut self, metrics: &ExecutionMetrics, test_name: &str) {
        if let Some(new_time) = &metrics.time {
            if self.slowest_test.is_none_or(|(time, _)| new_time > time) {
                self.slowest_test = Some((*new_time, test_name.to_string()));
            }
        }

        if let Some(new_memory) = &metrics.memory_kilobytes {
            if self.most_memory_used.is_none_or(|(memory, _)| new_memory > memory) {
                self.most_memory_used = Some((*new_memory, test_name.to_string()));
            }
        }
    }

    pub(crate) fn format_counts(&self, show_not_finished: bool) -> String {
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
            CountPart::new(if show_not_finished { self.total - self.processed } else { 0 }, "not finished").with_color(Yellow),
        ]
            .into_iter()
            .filter(|part| part.display_empty || part.count > 0)
            .map(|part| {
                format!("{} {}", part.count, part.get_text()).color(part.color).to_string()
            })
            .collect::<Vec<String>>()
            .join(", ")
    }

    pub(crate) fn get_errors(&mut self) -> &Vec<(String, TestError)> {
        self.test_errors.sort_unstable_by(|a, b| -> Ordering {
            human_sort::compare(&a.0, &b.0)
        });
        &self.test_errors
    }
}