mod args;
mod test_result;
mod testing_utils;

use std::{fs};
use std::cmp::Ordering;
use std::env::current_dir;
use std::fmt::{Write as FmtWrite};
use std::fs::{File, read_dir};
use std::path::{Path};
use std::sync::{Arc, Mutex};
use std::time::{Instant};
use atomic_counter::{AtomicCounter, RelaxedCounter};
use indicatif::{ParallelProgressIterator, ProgressState, ProgressStyle};
use lazy_static::lazy_static;
use rayon::iter::{IntoParallelRefIterator};
use rayon::prelude::*;
use tempfile::{tempdir};
use args::Args;
use clap::Parser;
use colored::Colorize;
use crate::test_result::TestResult;
use crate::testing_utils::{compile_cpp, generate_output, run_test};
use crate::TestResult::{Correct, Error, Incorrect};

lazy_static! {
    static ref SUCCESS_COUNT: RelaxedCounter = RelaxedCounter::new(0);
    static ref FAIL_COUNT: RelaxedCounter = RelaxedCounter::new(0);
}

fn main() {
	let args = Args::parse();
	let workspace_dir = current_dir().expect("The current directory is invalid!").to_str().expect("The current directory is invalid!").to_string();
	let input_dir: String = format!("{}/{}", &workspace_dir, args.r#in);
	let output_dir: String = format!("{}/{}", &workspace_dir, args.out);
	let tempdir = tempdir().expect("Failed to create temporary directory!");

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
	if !Path::new(&args.filename).is_file() { println!("{}", "The source code file does not exist".red()); return; }

	// Compiling
	let executable: String;
	match compile_cpp(Path::new(&args.filename).to_path_buf(), &tempdir) {
		Ok(result) => { executable = result }
		Err(error) => {
			println!("{}", "Compilation failed with the following errors:".red());
			println!("{}", error);
			return;
		}
	}

	// Progress bar styling
	let style: ProgressStyle = ProgressStyle::with_template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({eta})\n{correct} {incorrect}")
		.expect("Progress bar creation failed!")
		.with_key("eta", |state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{:.1}s", state.eta().as_secs_f64()).expect("Displaying the progress bar failed!"))
		.progress_chars("#>-")
		.with_key("correct", |_state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{}", format!("{} succeeded", &SUCCESS_COUNT.get()).green()).expect("Displaying the progress bar failed!"))
		.with_key("incorrect", |_state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{}", format!("{} failed", &FAIL_COUNT.get()).red()).expect("Displaying the progress bar failed!"));

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

	if input_files.is_empty() {
		println!("{}", "There are no files in the input directory with the provided file extension".red());
		return;
	}

	// Running tests / generating output
	let slowest_test = Arc::new(Mutex::new((-1 as f64, String::new())));
	let errors = Arc::new(Mutex::new(vec![]));
	let time_before_testing = Instant::now();
	input_files.par_iter().progress_with_style(style).for_each(|input| {
		let input_file_entry = input.as_ref().expect("Failed to acquire reference!");
		let input_file_path = input_file_entry.path();
		let input_file_path_str = input_file_path.to_string_lossy().to_string();
		let test_name = input_file_entry.path().file_stem().expect(&*format!("The input file {} is invalid!", input_file_path_str)).to_str().expect(&*format!("The input file {} is invalid!", input_file_path_str)).to_string();

		let test_time: f64;
		if args.generate {
			let input_file = File::open(input_file_path).expect(&*format!("Could not open input file {}", input_file_path_str));
			let output_file_path = format!("{}/{}{}", &output_dir, test_name, args.out_ext);
			let output_file = File::create(Path::new(&output_file_path)).expect("Failed to create output file!");

			match generate_output(&executable, input_file, output_file, &args.timeout) {
				Ok(time) => {
					test_time = time;
					SUCCESS_COUNT.inc();
				}
				Err((error, time)) => {
					FAIL_COUNT.inc();
					let clone = Arc::clone(&errors);
					clone.lock().expect("Failed to acquire mutex!").push(Error {test_name: test_name.clone(), error});
					test_time = time;
				}
			}
		}
		else {
			let (result, time) = run_test(&executable, input_file_path.as_path(), &output_dir, &test_name, &args.out_ext, &tempdir, &args.timeout);
			test_time = time;

			if let Correct { .. } = result {
				SUCCESS_COUNT.inc();
			}
			else {
				FAIL_COUNT.inc();
				let clone = Arc::clone(&errors);
				clone.lock().expect("Failed to acquire mutex!").push(result);
			}
		}

		let slowest_test_clone = Arc::clone(&slowest_test);
		let mut slowest_test_mutex = slowest_test_clone.lock().expect("Failed to acquire mutex!");
		if test_time > slowest_test_mutex.0 {
			*slowest_test_mutex = (test_time, test_name.clone());
		}
	});

	let testing_time = time_before_testing.elapsed().as_secs_f64();
	let slowest_test_clone = Arc::clone(&slowest_test);
	let errors_clone = Arc::clone(&errors);
	let slowest_test_mutex = slowest_test_clone.lock().expect("Failed to acquire mutex!");
	let mut errors_mutex = errors_clone.lock().expect("Failed to acquire mutex!");

	// Sorting the errors by name
	errors_mutex.sort_unstable_by(|a, b| -> Ordering {
		return human_sort::compare(&a.test_name(), &b.test_name());
	});

	// Printing the output
	match args.generate {
		true => {
			println!("Generation finished in {:.2}s with {} and {} (Slowest test: {} at {:.3}s)",
			         testing_time,
			         format!("{} successful generation{}", SUCCESS_COUNT.get(),
			                 if SUCCESS_COUNT.get() != 1 { "s" } else { "" }).green(),
			         format!("{} unsuccessful generation{}", FAIL_COUNT.get(),
			                 if FAIL_COUNT.get() != 1 { "s" } else { "" }).red(),
			         slowest_test_mutex.1,
			         slowest_test_mutex.0
			);
		}
		false => {
			println!("Testing finished in {:.2}s with {} and {} (Slowest test: {} at {:.3}s)",
			         testing_time,
			         format!("{} correct answer{}", SUCCESS_COUNT.get(),
			                 if SUCCESS_COUNT.get() != 1 { "s" } else { "" }).green(),
			         format!("{} incorrect answer{}", FAIL_COUNT.get(),
							 if FAIL_COUNT.get() != 1 { "s" } else { "" }).red(),
			         slowest_test_mutex.1,
			         slowest_test_mutex.0
			);
		}
	}

	// Printing errors if necessary
	if !errors_mutex.is_empty() {
		println!("Errors were found in the following tests:");

		for test_error in errors_mutex.iter() {
			println!("{}", test_error.to_string());
		}
	}
}