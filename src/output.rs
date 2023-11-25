use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use crate::test_summary::TestSummary;

pub(crate) fn get_initial_spinner() -> ProgressBar {
    let style = ProgressStyle::with_template("[{elapsed_precise}] [{spinner:.cyan}] {msg:.cyan}\n{warming}")
        .expect("Progress bar creation failed!")
        .with_key("warming", |_state: &ProgressState, w: &mut dyn fmt::Write| {
            write!(w, "{}", "Please wait while Toster is warming up".bright_black()).expect("Displaying the progress bar failed!");
        })
        .tick_strings(&[
            "    =",
            "=    ",
            "==   ",
            "===  ",
            " === ",
            "  ===",
            "   ==",
            "    =",
        ]);

    let bar = ProgressBar::new_spinner().with_style(style).with_message("Starting...");
    bar.enable_steady_tick(Duration::from_millis(100));
    bar
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