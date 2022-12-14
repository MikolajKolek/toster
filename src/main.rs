mod args;
mod test_result;
mod testing_utils;

use std::{fs, panic, process, thread};
use std::cmp::Ordering;
use std::fmt::{Write as FmtWrite};
use std::fs::{File, read_dir};
use std::panic::PanicInfo;
use std::path::{Path};
use std::sync::{Arc, atomic, Mutex};
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::time::{Duration, Instant};
use atomic_counter::{AtomicCounter, RelaxedCounter};
use indicatif::{ParallelProgressIterator, ProgressState, ProgressStyle};
use lazy_static::lazy_static;
use rayon::iter::{IntoParallelRefIterator};
use rayon::prelude::*;
use tempfile::{tempdir};
use args::Args;
use clap::Parser;
use colored::Colorize;
use human_panic::{handle_dump, Metadata, print_msg};
use is_executable::is_executable;
use crate::test_result::{ExecutionError, TestResult};
use crate::testing_utils::{compile_cpp, generate_output, run_test};
use crate::TestResult::{Correct, Error, Incorrect, NoOutputFile};
use once_cell::sync::{OnceCell};

lazy_static! {
    static ref SUCCESS_COUNT: RelaxedCounter = RelaxedCounter::new(0);
    static ref TIMED_OUT_COUNT: RelaxedCounter = RelaxedCounter::new(0);
    static ref INCORRECT_COUNT: RelaxedCounter = RelaxedCounter::new(0);
    static ref NON_ZER_RETURN_COUNT: RelaxedCounter = RelaxedCounter::new(0);
    static ref TERMINATED_COUNT: RelaxedCounter = RelaxedCounter::new(0);
    static ref NO_OUTPUT_COUNT: RelaxedCounter = RelaxedCounter::new(0);

	static ref SLOWEST_TEST: Arc<Mutex<(f64, String)>> = Arc::new(Mutex::new((-1 as f64, String::new())));
	static ref ERRORS: Arc<Mutex<Vec<TestResult>>> = Arc::new(Mutex::new(vec![]));
}

static TIME_BEFORE_TESTING: OnceCell<Instant> = OnceCell::new();
static TEST_COUNT: AtomicUsize = AtomicUsize::new(0);
static GENERATE: AtomicBool = AtomicBool::new(false);
static RECEIVED_CTRL_C: AtomicBool = AtomicBool::new(false);

// For whatever reason, the setup_panic!() macro doesn't seem to work, so I just made it a function
fn setup_panic() {
	match std::env::var("RUST_BACKTRACE") {
		Err(_) => {
			let meta = Metadata {
				version: env!("CARGO_PKG_VERSION").into(),
				name: env!("CARGO_PKG_NAME").into(),
				authors: env!("CARGO_PKG_AUTHORS").replace(":", ", ").into(),
				homepage: env!("CARGO_PKG_HOMEPAGE").into(),
			};

			panic::set_hook(Box::new(move |info: &PanicInfo| {
				let file_path = handle_dump(&meta, info);
				print_msg(file_path, &meta)
					.expect("human-panic: printing error message to console failed");
				process::exit(-1)
			}));
		}
		Ok(_) => {}
	}
}

fn format_error_counts() -> String {
	[
		(TIMED_OUT_COUNT.get(), "timed out"),
		(INCORRECT_COUNT.get(), if INCORRECT_COUNT.get() > 1 {"wrong answers"} else {"wrong answer"}),
		(NON_ZER_RETURN_COUNT.get(), if NON_ZER_RETURN_COUNT.get() > 1 {"non-zero return codes"} else {"non-zero return code"}),
		(TERMINATED_COUNT.get(), if TERMINATED_COUNT.get() > 1 {"terminated with errors"} else {"terminated with error"}),
		(NO_OUTPUT_COUNT.get(), if NO_OUTPUT_COUNT.get() > 1 {"without output files"} else {"without output file"}),
	]
		.into_iter()
		.filter(|(count, _)| count > &0)
		.map(|(count, label)| format!("{} {}", count.to_string().red(), label.to_string().red()))
		.collect::<Vec<String>>()
		.join(", ")
}

fn print_output(stopped_early: bool) {
	let slowest_test_clone = Arc::clone(&SLOWEST_TEST);
	let errors_clone = Arc::clone(&ERRORS);
	let slowest_test_mutex = slowest_test_clone.lock().expect("Failed to acquire mutex!");
	let mut errors_mutex = errors_clone.lock().expect("Failed to acquire mutex!");

	let testing_time = TIME_BEFORE_TESTING.get().expect("Time before testing not initialized!").elapsed().as_secs_f64();
	let tested_count = SUCCESS_COUNT.get() + TIMED_OUT_COUNT.get() + INCORRECT_COUNT.get() + NON_ZER_RETURN_COUNT.get() + TERMINATED_COUNT.get() + NO_OUTPUT_COUNT.get();
	let not_tested_count = &TEST_COUNT.load(atomic::Ordering::Acquire) - tested_count;

	let error_counts = format_error_counts();
	let error_text = format!("{}{}", if !error_counts.is_empty() {", "} else {""}, error_counts);
	let not_finished_text = if not_tested_count > 0 {format!(", {}", (not_tested_count.to_string() + " not finished").yellow())} else {"".to_string()};

	if stopped_early {
		println!();
	}

	// Printing the output
	match GENERATE.load(atomic::Ordering::Acquire) {
		true => {
			println!("Generation {} {:.2}s (Slowest test: {} at {:.3}s)\nResults: {}{}{}",
			         if stopped_early {"stopped after"} else {"finished in"},
			         testing_time,
			         slowest_test_mutex.1,
			         slowest_test_mutex.0,
			         format!("{} successful", SUCCESS_COUNT.get()).green(),
			         error_text,
			         not_finished_text
			);
		}
		false => {
			println!("Testing {} {:.2}s (Slowest test: {} at {:.3}s)\nResults: {}{}{}",
			         if stopped_early {"stopped after"} else {"finished in"},
			         testing_time,
			         slowest_test_mutex.1,
			         slowest_test_mutex.0,
			         format!("{} correct", SUCCESS_COUNT.get()).green(),
			         error_text,
			         not_finished_text
			);
		}
	}

	// Sorting the errors by name
	errors_mutex.sort_unstable_by(|a, b| -> Ordering {
		return human_sort::compare(&a.test_name(), &b.test_name());
	});

	// Printing errors if necessary
	if !errors_mutex.is_empty() {
		println!("Errors were found in the following tests:");

		for test_error in errors_mutex.iter() {
			println!("{}", test_error.to_string());
		}
	}

	process::exit(0);
}

fn main() {
	setup_panic();
	ctrlc::set_handler(move || {
		RECEIVED_CTRL_C.store(true, atomic::Ordering::Release);
		print_output(true)
	}).expect("Error setting Ctrl-C handler");

	let args = Args::parse();
	GENERATE.store(args.generate, atomic::Ordering::Release);
	let tempdir = tempdir().expect("Failed to create temporary directory!");
	let input_dir: String = args.io.clone().unwrap_or(args.r#in);
	let output_dir: String = args.io.clone().unwrap_or(args.out);

	// Making sure that the input and output directories as well as the source code file exist
	if !Path::new(&output_dir).is_dir() {
		if args.generate {
			fs::create_dir(&output_dir).expect("Failed to create output directory!");
		}
		else {
			println!("{}", "The output directory does not exist".red());
			return;
		}
	}
	if !Path::new(&input_dir).is_dir() { println!("{}", "The input directory does not exist".red()); return; }
	if !Path::new(&args.filename).is_file() { println!("{}", "The provided file does not exist".red()); return; }

	if !args.compile_command.contains("<IN>") || !args.compile_command.contains("<OUT>") {
		println!("{}", "The compile command is invalid:".red());

		if !args.compile_command.contains("<IN>") {
			println!("{}", "- The <IN> argument is missing (read \"toster -h\" for more info)".red());
		}
		if !args.compile_command.contains("<OUT>") {
			println!("{}", "- The <OUT> argument is missing (read \"toster -h\" for more info)".red());
		}

		return;
	}

	// Compiling
	let executable: String;
	if is_executable(&args.filename) {
		executable = tempdir.path().join(format!("{}.o", Path::new(&args.filename).file_name().expect("The provided filename is invalid!").to_str().expect("The provided filename is invalid!"))).to_str().expect("The provided filename is invalid!").to_string();
		fs::copy(&args.filename, &executable).expect("The provided filename is invalid!");
	}
	else {
		match compile_cpp(Path::new(&args.filename).to_path_buf(), &tempdir, args.compile_timeout, &args.compile_command) {
			Ok(result) => { executable = result }
			Err(error) => {
				println!("{}", "Compilation failed with the following errors:".red());
				println!("{}", error);
				return;
			}
		}
	}

	// Progress bar styling
	let style: ProgressStyle = ProgressStyle::with_template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({eta})\n{correct} {incorrect} {ctrlc}")
		.expect("Progress bar creation failed!")
		.with_key("eta", |state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{:.1}s", state.eta().as_secs_f64()).expect("Displaying the progress bar failed!"))
		.progress_chars("#>-")
		.with_key("correct", |_state: &ProgressState, w: &mut dyn FmtWrite|
			write!(w, "{}", format!("{} {}", &SUCCESS_COUNT.get(), if GENERATE.load(atomic::Ordering::Acquire) {"successful"} else {if SUCCESS_COUNT.get() != 1 {"correct answers"} else {"correct answer"}}).green()).expect("Displaying the progress bar failed!")
		)
		.with_key("incorrect", |_state: &ProgressState, w: &mut dyn FmtWrite|
			write!(w, "{}", format_error_counts()).expect("Displaying the progress bar failed!")
		)
		.with_key("ctrlc", |_state: &ProgressState, w: &mut dyn FmtWrite|
			write!(w, "{}", "(Press Ctrl+C to stop testing and print current results)".bright_black()).expect("Displaying the progress bar Ctrl+C message failed!")
		);

	// Filtering out input files
	let mut input_files = read_dir(&input_dir).expect("Cannot open input directory!").collect::<Vec<_>>();
	input_files.retain(|input| {
		let input_path = input.as_ref().expect("Failed to acquire reference!").path();
		let extension = input_path.extension();

		return match extension {
			None => {false}
			Some(ext) => { ".".to_owned() + &ext.to_str().unwrap_or("") == args.in_ext }
		};
	});
	TEST_COUNT.store(input_files.len(), atomic::Ordering::Release);

	if input_files.is_empty() {
		println!("{}", "There are no files in the input directory with the provided file extension".red());
		return;
	}

	TIME_BEFORE_TESTING.set(Instant::now()).expect("Couldn't set time before testing!");
	// Running tests / generating output
	input_files.par_iter().progress_with_style(style).for_each(|input| {
		let input_file_entry = input.as_ref().expect("Failed to acquire reference!");
		let input_file_path = input_file_entry.path();
		let input_file_path_str = input_file_path.to_string_lossy().to_string();
		let test_name = input_file_entry.path().file_stem().expect(&*format!("The input file {} is invalid!", input_file_path_str)).to_str().expect(&*format!("The input file {} is invalid!", input_file_path_str)).to_string();

		let mut test_time: f64 = f64::MAX;
		if args.generate {
			let input_file = File::open(input_file_path).expect(&*format!("Could not open input file {}", input_file_path_str));
			let output_file_path = format!("{}/{}{}", &output_dir, test_name, args.out_ext);
			let output_file = File::create(Path::new(&output_file_path)).expect("Failed to create output file!");

			let result = generate_output(&executable, input_file, output_file, &args.timeout);
			if !RECEIVED_CTRL_C.load(atomic::Ordering::Acquire) {
				match result {
					Ok(time) => {
						test_time = time;
						SUCCESS_COUNT.inc();
					}
					Err((error, time)) => {
						match error {
							ExecutionError::TimedOut => { TIMED_OUT_COUNT.inc(); }
							ExecutionError::NonZeroReturn(_) => { NON_ZER_RETURN_COUNT.inc(); }
							ExecutionError::Terminated(_) => { TERMINATED_COUNT.inc(); }
						}
						let clone = Arc::clone(&ERRORS);
						clone.lock().expect("Failed to acquire mutex!").push(Error { test_name: test_name.clone(), error });
						test_time = time;
					}
				}
			}
			else {
				thread::sleep(Duration::from_secs(u64::MAX));
			}
		}
		else {
			let (result, time) = run_test(&executable, input_file_path.as_path(), &output_dir, &test_name, &args.out_ext, &tempdir, &args.timeout);
			test_time = time;

			if !RECEIVED_CTRL_C.load(atomic::Ordering::Acquire) {
				match result {
					Correct { .. } => { SUCCESS_COUNT.inc(); }
					Incorrect { .. } => { INCORRECT_COUNT.inc(); }
					Error { error: ExecutionError::NonZeroReturn(_), .. } => { NON_ZER_RETURN_COUNT.inc(); }
					Error { error: ExecutionError::TimedOut, .. } => { TIMED_OUT_COUNT.inc(); }
					Error { error: ExecutionError::Terminated(_), .. } => { TERMINATED_COUNT.inc(); }
					NoOutputFile { .. } => { NO_OUTPUT_COUNT.inc(); }
				}

				if !result.is_correct() {
					let clone = Arc::clone(&ERRORS);
					clone.lock().expect("Failed to acquire mutex!").push(result);
				}
			}
			else {
				thread::sleep(Duration::from_secs(u64::MAX));
			}
		}

		if !RECEIVED_CTRL_C.load(atomic::Ordering::Acquire) {
			let slowest_test_clone = Arc::clone(&SLOWEST_TEST);
			let mut slowest_test_mutex = slowest_test_clone.lock().expect("Failed to acquire mutex!");
			if test_time > slowest_test_mutex.0 {
				*slowest_test_mutex = (test_time, test_name.clone());
			}
		}
		else {
			thread::sleep(Duration::from_secs(u64::MAX));
		}
	});

	print_output(false)
}
