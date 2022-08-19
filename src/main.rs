mod args;
mod test_error;

use std::{fs};
use std::env::current_dir;
use std::fmt::{Write as FmtWrite};
use std::fs::{DirEntry, File, read_dir};
use std::path::Path;
use std::process::{Command};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use atomic_counter::{AtomicCounter, RelaxedCounter};
use indicatif::{ParallelProgressIterator, ProgressState, ProgressStyle};
use lazy_static::lazy_static;
use rayon::iter::{IntoParallelRefIterator};
use rayon::prelude::*;
use tempfile::tempdir;
use wait_timeout::ChildExt;
use args::Args;
use clap::Parser;
use colored::Colorize;
use crate::test_error::TestError;
use crate::TestError::{NoOutputFile, TimedOut, Incorrect};

lazy_static! {
    static ref CORRECT: RelaxedCounter = RelaxedCounter::new(0);
    static ref INCORRECT: RelaxedCounter = RelaxedCounter::new(0);
}

fn main() {
	let args = Args::parse();
	let workspace_dir = current_dir().expect("The current directory is invalid!").to_str().expect("The current directory is invalid!").to_string();
	let input_dir: String = format!("{}/{}", &workspace_dir, args.r#in);
	let output_dir: String = format!("{}/{}", &workspace_dir, args.out);
	let executable = format!("{}.o", Path::new(&args.filename).file_stem().expect("The provided filename is invalid!").to_str().expect("The provided filename is invalid!"));
	let tempdir = tempdir().expect("Failed to create temporary directory");

	if !Path::new(&input_dir).is_dir() { println!("{}", "The input directory does not exist".red()); return; }
	if args.generate && !Path::new(&output_dir).is_dir() {
		fs::create_dir(&output_dir).expect("Failed to create output directory!");
	}
	else {
		if !Path::new(&output_dir).is_dir() { println!("{}", "The output directory does not exist".red());return; }
	}
	if !Path::new(&args.filename).is_file() { println!("{}", "The source code file does not exist".red()); return; }

	let compilation_tempfile_path = tempdir.path().join(format!("{}.out", executable));
	let compilation_tempfile = File::create(&compilation_tempfile_path).expect("Failed to create temporary file!");
	let time_before_compilation = Instant::now();
	Command::new("g++")
		.args(["-std=c++17", "-O3", "-static", &args.filename, "-o", &executable])
		.stderr(compilation_tempfile)
		.spawn().expect("g++ failed to start").wait_timeout(Duration::from_secs(10)).expect("Compilation took too long!");
	let gpp_out = fs::read_to_string(compilation_tempfile_path).expect("Failed to read temporary file!");
	if !gpp_out.is_empty() {
		println!("{}\n{}", "Compilation failed with the following errors:".red(), gpp_out);
		return;
	}
	else {
		println!("{}", format!("Compilation completed in {:.2}s", time_before_compilation.elapsed().as_secs_f64()).green())
	}

	let mut style: ProgressStyle = ProgressStyle::with_template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({eta})\n{correct} {incorrect}")
		.expect("Progress bar creation failed!")
		.with_key("eta", |state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{:.1}s", state.eta().as_secs_f64()).expect("Displaying the progress bar failed!"))
		.progress_chars("#>-");
	if !args.generate {
		style = style.with_key("correct", |_state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{}", format!("{} correct", &CORRECT.get()).green()).expect("Displaying the progress bar failed!"))
			.with_key("incorrect", |_state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{}", format!("{} incorrect", &INCORRECT.get()).red()).expect("Displaying the progress bar failed!"));
	}

	let slowest_test: Arc<Mutex<(f64, String)>> = Arc::new(Mutex::new((-1 as f64, String::new())));
	let errors: Arc<Mutex<Vec<TestError>>> = Arc::new(Mutex::new(vec![]));
	let before_testing = Instant::now();
	read_dir(&input_dir).expect("Cannot open input directory!").collect::<Vec<_>>().par_iter().progress_with_style(style).for_each(|input| {
		let file: &DirEntry = input.as_ref().expect("Failed to acquire reference!");
		let file_path = file.path().to_string_lossy().to_string();
		let test_name = file.path().file_stem().expect(&*format!("The input file {} is invalid!", file_path)).to_str().expect(&*format!("The input file {} is invalid!", file_path)).to_string();
		let input_file = File::open(file.path()).expect(&*format!("Could not open input file {}", file_path));
		let output_file = format!("{}/{}.out", &output_dir, test_name);

		let test_output_file_path = if args.generate {
			Path::new(&output_file).to_path_buf()
		}
		else  {
			tempdir.path().join(format!("{}.out", test_name))
		};
		let test_output_file = File::create(&test_output_file_path).expect("Failed to create temporary file!");

		let start = Instant::now();
		let mut child = Command::new(format!("./{}", &executable))
			.stdout(test_output_file)
			.stdin(input_file)
			.spawn().expect("Failed to run compiled file!");
		let timed_out = match child.wait_timeout(Duration::from_secs(args.timeout)).unwrap() {
			Some(_) => false,
			None => {
				child.kill().unwrap();
				true
			}
		};
		let test_time = start.elapsed().as_secs_f64();
		let output: String = fs::read_to_string(&test_output_file_path).expect("Failed to read temporary file!");

		let clone = Arc::clone(&slowest_test);
		let mut slowest_test_mutex = clone.lock().expect("Failed to acquire mutex!");
		if test_time > slowest_test_mutex.0 {
			*slowest_test_mutex = (test_time, test_name.clone());
		}

		if timed_out {
			if args.generate {
				fs::remove_file(&test_output_file_path).expect("Failed to remove output file from timed-out test");
			}

			INCORRECT.inc();
			let clone = Arc::clone(&errors);
			clone.lock().expect("Failed to acquire mutex!").push(TimedOut { test_name });
			return;
		}

		if !args.generate {
			if !Path::new(&output_file).is_file() {
				INCORRECT.inc();
				let clone = Arc::clone(&errors);
				clone.lock().expect("Failed to acquire mutex!").push(/*"".to_string(), "".to_string(), "".to_string(), */NoOutputFile {test_name});
				return;
			}

			let output_file_contents = fs::read_to_string(Path::new(&output_file)).expect("Failed to read output file!");
			if output.split_whitespace().collect::<Vec<&str>>() != output_file_contents.split_whitespace().collect::<Vec<&str>>() {
				INCORRECT.inc();
				let clone = Arc::clone(&errors);
				clone.lock().expect("Failed to acquire mutex!").push(/*test_name, output, output_file_contents,*/ /*"".to_string(), "".to_string(), "".to_string(), */Incorrect {test_name, correct_answer: output_file_contents, incorrect_answer: output });
			}
			else {
				CORRECT.inc();
			}
		}
	});

	let slowest_test_clone = Arc::clone(&slowest_test);
	let slowest_test_mutex = slowest_test_clone.lock().expect("Failed to acquire mutex!");
	let errors_clone = Arc::clone(&errors);
	let errors_mutex = errors_clone.lock().expect("Failed to acquire mutex!");
	if !args.generate {
		println!("Testing finished in {:.2}s with {} and {}: (Slowest test: {} at {:.3}s)",
		         before_testing.elapsed().as_secs_f64(),
		         format!("{} correct answers", CORRECT.get()).green(),
		         format!("{} incorrect answers", INCORRECT.get()).red(),
		         slowest_test_mutex.1,
		         slowest_test_mutex.0
		);

		if !errors_mutex.is_empty() {
			println!("Errors were found in the following tests:");

			for test_error in errors_mutex.iter() {
				println!("{}", test_error.to_string());
			}
		}
	}
	else {
		println!("Program finished in {:.2}s (Slowest test: {} at {:.3}s)",
		         before_testing.elapsed().as_secs_f64(),
		         slowest_test_mutex.1,
		         slowest_test_mutex.0
		);

		if !errors_mutex.is_empty() {
			println!("Errors were found in the following tests:");

			for test_error in errors_mutex.iter() {
				println!("{}", test_error.to_string());
			}
		}
	}
}