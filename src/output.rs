use std::{fmt, thread};
use std::marker::PhantomData;
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::thread::{Scope, ScopedJoinHandle};
use std::time::Duration;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use crate::formatted_error::FormattedError;
use crate::test_summary::TestSummary;

pub(crate) struct InitialSpinner<'scope, 'data: 'scope, 'env: 'scope + 'data> {
    data: &'data InitialSpinnerData,
    scope: &'scope Scope<'scope, 'data>,
    env: PhantomData<&'env mut &'env ()>,
}

struct InitialSpinnerData {
    bar: ProgressBar,
    jobs: Mutex<Vec<InitialSpinnerJob>>,
}

struct InitialSpinnerJob {
    name: &'static str,
    state: JobState,
}

enum JobState {
    Working,
    Done,
    Failed,
}

impl<'scope, 'data, 'env> InitialSpinner<'scope, 'data, 'env> {
    pub(crate) fn add_job<T, F>(&mut self, name: &'static str, f: F) -> ScopedJoinHandle<'scope, T>
    where
        T: Send + 'scope,
        F: FnOnce() -> Result<T, FormattedError> + Send + 'scope,
    {
        let data = self.data;
        let job_id = data.add_job(name);
        self.scope.spawn(move || {
            let result = f();
            let result = match result {
                Ok(result) => {
                    data.set_job_state(job_id, JobState::Done);
                    result
                },
                Err(error) => {
                    data.set_job_state(job_id, JobState::Failed);
                    data.bar.finish();
                    println!("\n");
                    exit_with_error(error)
                }
            };
            result
        })
    }
}

/// Displays warming up animation and lists of running or completed startup jobs.
/// `callback` is called with a reference to an `InitialSpinner` instance.
/// Use `InitialSpinner::add_job()` to start new startup job. The jobs are run concurrently.
///
/// All jobs should return a `Result<T, FormattedError>`.
/// If a job exits early with an error ``exit_with_error` is called,
/// so errors are handled as soon as possible.
///
/// `start_initial_spinner` calls `std::thread::scope` under the hood so,
/// jobs are automatically waited for before `start_initial_spinner` returns.
pub(crate) fn start_initial_spinner<'env, T, F>(callback: F) -> T
where
    F: for<'spinner, 'scope, 'data> FnOnce(&'spinner mut InitialSpinner<'scope, 'data, 'env>) -> T,
{
    let style = ProgressStyle::with_template("[{spinner:.cyan}] {msg}\n{warming}")
        .expect("Progress bar creation failed!")
        .with_key("warming", |_state: &ProgressState, w: &mut dyn fmt::Write| {
            if !_state.is_finished() {
                write!(w, "{}", "Please wait while Toster is warming up".bright_black()).expect("Displaying the progress bar failed!");
            }
        })
        .tick_strings(&[
            "=       ",
            "==      ",
            "===     ",
            " ===    ",
            "  ===   ",
            "   ===  ",
            "    === ",
            "     ===",
            "      ==",
            "       =",
            "xxxxxxxx",
        ]);
    let bar = ProgressBar::new_spinner().with_style(style).with_message("Starting...");
    bar.enable_steady_tick(Duration::from_millis(100));

    let data = InitialSpinnerData {
        bar,
        jobs: Mutex::new(vec![]),
    };

    thread::scope(|scope| {
        let mut spinner = InitialSpinner {
            data: &data,
            scope,
            env: PhantomData,
        };
        let result = callback(&mut spinner);
        spinner.data.bar.finish_and_clear();
        result
    })
}

impl InitialSpinnerData {
    fn update_message(&self) {
        let jobs = self.jobs.lock().unwrap();
        let parts: Vec<String> = jobs.iter().map(|job| {
            use JobState::*;
            match &job.state {
                Working => job.name.to_string(),
                Done => job.name.green().to_string(),
                Failed => job.name.red().to_string(),
            }
        }).collect();
        self.bar.set_message(parts.join(&", ".bright_black().to_string()));
    }

    fn add_job(&self, name: &'static str) -> usize {
        let mut jobs = self.jobs.lock().unwrap();
        let index = jobs.len();
        jobs.push(InitialSpinnerJob {
            name,
            state: JobState::Working,
        });
        drop(jobs);
        self.update_message();
        index
    }

    fn set_job_state(&self, job_id: usize, state: JobState) {
        let mut jobs = self.jobs.lock().unwrap();
        jobs[job_id].state = state;
        drop(jobs);
        self.update_message();
    }
}

pub(crate) fn get_progress_bar(test_summary: &Arc<Mutex<Option<TestSummary>>>) -> ProgressBar {
    let test_count = test_summary.lock().expect("Failed to lock test summary mutex").as_ref().unwrap().total;
    let test_summary = test_summary.clone();
    let style = ProgressStyle::with_template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({eta})\n{counts} {ctrlc}")
        .expect("Progress bar creation failed!")
        .with_key("eta", |state: &ProgressState, w: &mut dyn fmt::Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).expect("Displaying the progress bar failed!"))
        .progress_chars("#>-")
        .with_key("counts", move |_state: &ProgressState, w: &mut dyn fmt::Write| {
            write!(w, "{}", test_summary.lock().expect("Failed to lock test summary mutex").as_ref().unwrap().format_counts(false)).expect("Displaying the progress bar failed!")
        })
        .with_key("ctrlc", |_state: &ProgressState, w: &mut dyn fmt::Write|
            write!(w, "{}", "(Press Ctrl+C to stop testing and print current results)".bright_black()).expect("Displaying the progress bar Ctrl+C message failed!")
        );

    ProgressBar::new(test_count as u64).with_style(style)
}

fn exit_with_error(error: FormattedError) -> ! {
    println!("{}", error);
    exit(1)
}

pub(crate) fn print_output(stopped_early: bool, test_summary: &mut Option<TestSummary>) {
    let Some(test_summary) = test_summary else {
        println!("{}", "Toster was stopped before testing could start".red());
        return;
    };

    if stopped_early {
        println!();
    }

    let additional_info = match (&test_summary.slowest_test, &test_summary.most_memory_used) {
        (None, None) => "".to_string(),
        (Some((duration, slowest_test_name)), None) => format!(
            " (Slowest test: {} at {:.3}s)",
            slowest_test_name, duration.as_secs_f32(),
        ),
        (None, Some((memory, most_memory_test_name))) => format!(
            " (Most memory used: {} at {:.3}KiB)",
            most_memory_test_name, memory,
        ),
        (Some((duration, slowest_test_name)), Some((memory, most_memory_test_name))) => format!(
            " (Slowest test: {} at {:.3}s, most memory used: {} at {}KiB)",
            slowest_test_name, duration.as_secs_f32(),
            most_memory_test_name, memory,
        ),
    };

    println!(
        "{} {} {:.2}s{}\nResults: {}",
        if test_summary.generate_mode { "Generating" } else { "Testing" },
        if stopped_early {"stopped after"} else {"finished in"},
        test_summary.start_time.elapsed().as_secs_f64(),
        additional_info,
        test_summary.format_counts(true),
    );

    let incorrect_results = test_summary.get_errors();
    if !incorrect_results.is_empty() {
        println!("Errors were found in the following tests:");

        for (test_name, error) in incorrect_results.iter() {
            println!("{}", error.to_string(test_name));
        }
    }
}