use std::{fmt, thread};
use std::sync::{Arc, Mutex};
use std::thread::{Scope, ScopedJoinHandle};
use std::time::Duration;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use crate::test_summary::TestSummary;

pub(crate) struct InitialSpinner<'scope, 'env: 'scope> {
    data: InitialSpinnerData,
    scope: &'scope Scope<'scope, 'env>,
}

#[derive(Clone)]
struct InitialSpinnerData {
    bar: ProgressBar,
    jobs: Arc<Mutex<Vec<InitialSpinnerJob>>>,
}

struct InitialSpinnerJob {
    name: &'static str,
    done: bool,
}

impl<'scope, 'env> InitialSpinner<'scope, 'env> {
    pub(crate) fn add_job<T, F>(&mut self, name: &'static str, f: F) -> ScopedJoinHandle<'scope, T>
    where
        T: Send + 'scope,
        F: FnOnce() -> T + Send + 'scope,
    {
        let data = self.data.clone();
        let job_id = data.add_job(name);
        self.scope.spawn(move || {
            let result = f();
            data.finish_job(job_id);
            result
        })
    }
}

pub(crate) fn start_initial_spinner<'env, T, F>(callback: F) -> T
where
    F: for<'scope> FnOnce(InitialSpinner<'scope, 'env>) -> T,
{
    let style = ProgressStyle::with_template("[{spinner:.cyan}] {msg}\n{warming}")
        .expect("Progress bar creation failed!")
        .with_key("warming", |_state: &ProgressState, w: &mut dyn fmt::Write| {
            write!(w, "{}", "Please wait while Toster is warming up".bright_black()).expect("Displaying the progress bar failed!");
        })
        .tick_strings(&[
            "       =",
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
        ]);
    let bar = ProgressBar::new_spinner().with_style(style).with_message("Starting...");
    bar.enable_steady_tick(Duration::from_millis(100));

    let data = InitialSpinnerData {
        bar: bar.clone(),
        jobs: Arc::new(Mutex::new(vec![])),
    };

    thread::scope(|scope| {
        let spinner = InitialSpinner {
            data,
            scope,
        };
        let result = callback(spinner);
        bar.finish_and_clear();
        result
    })
}

impl InitialSpinnerData {
    fn update_message(&self) {
        let jobs = self.jobs.lock().unwrap();
        let parts: Vec<String> = jobs.iter().map(|job| {
            if job.done {
                job.name.green().to_string()
            } else {
                job.name.to_string()
            }
        }).collect();
        self.bar.set_message(parts.join(&", ".bright_black().to_string()));
    }

    fn add_job(&self, name: &'static str) -> usize {
        let mut jobs = self.jobs.lock().unwrap();
        let index = jobs.len();
        jobs.push(InitialSpinnerJob {
            name,
            done: false,
        });
        drop(jobs);
        self.update_message();
        index
    }

    fn finish_job(&self, job_id: usize) {
        let mut jobs = self.jobs.lock().unwrap();
        jobs[job_id].done = true;
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