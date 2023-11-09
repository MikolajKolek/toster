mod args;
mod test_result;
mod testing_utils;

use std::{fs, panic, process, thread};
use std::cmp::Ordering;
use std::ffi::OsStr;
use std::fmt::Write as FmtWrite;
use std::fs::{File, read_dir};
use std::panic::PanicInfo;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::atomic::Ordering::{Acquire, Release};
use std::time::{Duration, Instant};
use atomic_counter::{AtomicCounter, RelaxedCounter};
use clap::Parser;
use colored::Colorize;
use human_panic::{handle_dump, print_msg};
use indicatif::{ParallelProgressIterator, ProgressState, ProgressStyle};
use is_executable::is_executable;
use lazy_static::lazy_static;
use rayon::iter::IntoParallelRefIterator;
use rayon::prelude::*;
use tempfile::tempdir;
use which::which;
use args::Args;
use crate::test_result::{ExecutionError, TestResult};
use crate::testing_utils::{compile_cpp, fill_tempfile_pool, generate_output_default, init_sio2jail, run_test};
use crate::TestResult::{Correct, Error, Incorrect, NoOutputFile};

lazy_static! {
    static ref SUCCESS_COUNT: RelaxedCounter = RelaxedCounter::new(0);
	static ref INCORRECT_COUNT: RelaxedCounter = RelaxedCounter::new(0);
    static ref TIMED_OUT_COUNT: RelaxedCounter = RelaxedCounter::new(0);
	static ref INVALID_OUTPUT_COUNT: RelaxedCounter = RelaxedCounter::new(0);
    static ref MEMORY_LIMIT_EXCEEDED_COUNT: RelaxedCounter = RelaxedCounter::new(0);
    static ref RUNTIME_ERROR_COUNT: RelaxedCounter = RelaxedCounter::new(0);
    static ref NO_OUTPUT_FILE_COUNT: RelaxedCounter = RelaxedCounter::new(0);
	static ref SIO2JAIL_ERROR_COUNT: RelaxedCounter = RelaxedCounter::new(0);

    static ref SLOWEST_TEST: Arc<Mutex<(f64, String)>> = Arc::new(Mutex::new((-1 as f64, String::new())));
    static ref MOST_MEMORY_USED: Arc<Mutex<(i64, String)>> = Arc::new(Mutex::new((-1, String::new())));
    static ref ERRORS: Arc<Mutex<Vec<TestResult>>> = Arc::new(Mutex::new(vec![]));
}

static TIME_BEFORE_TESTING: OnceLock<Instant> = OnceLock::new();
static TEST_COUNT: AtomicUsize = AtomicUsize::new(0);
static GENERATE: AtomicBool = AtomicBool::new(false);

static RECEIVED_CTRL_C: AtomicBool = AtomicBool::new(false);
static PANICKING: AtomicBool = AtomicBool::new(false);

fn format_error_counts() -> String {
	[
		(INCORRECT_COUNT.get(), if INCORRECT_COUNT.get() > 1 { "wrong answers" } else { "wrong answer" }, ),
		(TIMED_OUT_COUNT.get(), "timed out"),
		(INVALID_OUTPUT_COUNT.get(), if INVALID_OUTPUT_COUNT.get() > 1 { "invalid outputs" } else { "invalid output" }),
		(MEMORY_LIMIT_EXCEEDED_COUNT.get(), "out of memory"),
		(RUNTIME_ERROR_COUNT.get(), if RUNTIME_ERROR_COUNT.get() > 1 { "runtime errors" } else { "runtime error" }),
		(NO_OUTPUT_FILE_COUNT.get(), if NO_OUTPUT_FILE_COUNT.get() > 1 { "without output files" } else { "without output file" }),
		(SIO2JAIL_ERROR_COUNT.get(), if SIO2JAIL_ERROR_COUNT.get() > 1 { "sio2jail errors" } else { "sio2jail error" })
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
	let most_memory_clone = Arc::clone(&MOST_MEMORY_USED);
	let slowest_test_mutex = slowest_test_clone.lock().expect("Failed to acquire mutex!");
	let mut errors_mutex = errors_clone.lock().expect("Failed to acquire mutex!");
	let most_memory_mutex = most_memory_clone.lock().expect("Failed to acquire mutex!");

	if TIME_BEFORE_TESTING.get().is_none() {
		println!("{}", "Toster was stopped before testing could start".red());
		process::exit(0);
	}

	let testing_time = TIME_BEFORE_TESTING.get().unwrap().elapsed().as_secs_f64();
	let tested_count = SUCCESS_COUNT.get() + TIMED_OUT_COUNT.get() + INCORRECT_COUNT.get() + MEMORY_LIMIT_EXCEEDED_COUNT.get() + INVALID_OUTPUT_COUNT.get() + RUNTIME_ERROR_COUNT.get() + NO_OUTPUT_FILE_COUNT.get() + SIO2JAIL_ERROR_COUNT.get();
	let not_tested_count = &TEST_COUNT.load(Acquire) - tested_count;

	let error_counts = format_error_counts();
	let error_text = format!("{}{}", if !error_counts.is_empty() {", "} else {""}, error_counts);
	let not_finished_text = if not_tested_count > 0 {format!(", {}", (not_tested_count.to_string() + " not finished").yellow())} else {"".to_string()};

	if stopped_early {
		println!();
	}

	let mut additional_info = String::new();
	if slowest_test_mutex.0 != -1 as f64 {
		additional_info = format!(" (Slowest test: {} at {:.3}s{})",
		                          slowest_test_mutex.1,
		                          slowest_test_mutex.0,
		                          if most_memory_mutex.0 != -1 { format!(", most memory used: {} at {}KiB", most_memory_mutex.1, most_memory_mutex.0) } else { String::new() }
		)
	}

	// Printing the output
	match GENERATE.load(Acquire) {
		true => {
			println!("Generation {} {:.2}s{}\nResults: {}{}{}",
			         if stopped_early {"stopped after"} else {"finished in"},
			         testing_time,
			         additional_info,
			         format!("{} successful", SUCCESS_COUNT.get()).green(),
			         error_text,
			         not_finished_text
			);
		}
		false => {
			println!("Testing {} {:.2}s{}\nResults: {}{}{}",
			         if stopped_early {"stopped after"} else {"finished in"},
			         testing_time,
			         additional_info,
			         format!("{} correct", SUCCESS_COUNT.get()).green(),
			         error_text,
			         not_finished_text
			);
		}
	}

	// Printing errors if necessary
	if !errors_mutex.is_empty() {
		// Sorting the errors by name
		errors_mutex.sort_unstable_by(|a, b| -> Ordering {
			return human_sort::compare(&a.test_name(), &b.test_name());
		});

		println!("Errors were found in the following tests:");

		for test_error in errors_mutex.iter() {
			println!("{}", test_error.to_string());
		}
	}

	process::exit(0);
}

fn setup_panic() {
	match human_panic::PanicStyle::default() {
		human_panic::PanicStyle::Debug => {}
		human_panic::PanicStyle::Human => {
			let meta = human_panic::metadata!();

			panic::set_hook(Box::new(move |info: &PanicInfo| {
				if !PANICKING.load(Acquire) {
					PANICKING.store(true, Release);

					let file_path = handle_dump(&meta, info);
					print_msg(file_path, &meta).expect("human-panic: printing error message to console failed");
					process::exit(0);
				}
				else {
					thread::sleep(Duration::from_secs(u64::MAX));
				}
			}));
		}
	}
}

fn main() {
	setup_panic();
	ctrlc::set_handler(move || {
		RECEIVED_CTRL_C.store(true, Release);
		print_output(true)
	}).expect("Error setting Ctrl-C handler");

	let tempdir = tempdir().expect("Failed to create temporary directory!");
	fill_tempfile_pool(&tempdir);

	let args = Args::parse();
	GENERATE.store(args.generate, Release);
	let input_dir: String = args.io.clone().unwrap_or(args.r#in);
	let output_dir: String = args.io.clone().unwrap_or(args.out);

	#[allow(unused_assignments)]
	let mut sio2jail = false;
	#[allow(unused_assignments)]
	let mut memory_limit = 0;
	#[cfg(all(target_os = "linux", target_arch = "x86_64"))] {
		sio2jail = args.sio2jail;
		memory_limit = args.memory_limit.unwrap_or(0);
	}

	if memory_limit != 0 && !sio2jail {
		sio2jail = true;
	}
	if sio2jail && args.generate {
		println!("{}", "You can't have the --generate and --sio2jail flags on at the same time.".red());
		return;
	}
	if sio2jail && memory_limit == 0 {
		memory_limit = 1048576;
	}
	if sio2jail && !init_sio2jail() {
		return;
	}

	// Making sure that the input and output directories as well as the source code file exist
	if args.io.is_some() && !Path::new(&args.io.unwrap()).is_dir() {
		println!("{}", "The input/output directory does not exist".red());
		return;
	}
	if !Path::new(&input_dir).is_dir() {
		println!("{}", "The input directory does not exist".red());
		return;
	}
	if !Path::new(&output_dir).is_dir() {
		if args.generate {
			fs::create_dir(&output_dir).expect("Failed to create output directory!");
		}
		else {
			println!("{}", "The output directory does not exist".red());
			return;
		}
	}
	if !Path::new(&args.filename).is_file() {
		println!("{}", "The provided file does not exist".red());
		return;
	}

	// Making sure the compile command is valid
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
	let extension = Path::new(&args.filename).extension().unwrap_or(OsStr::new("")).to_str().expect("Couldn't get the extension of the provided file");
	let executable: String;
	if !is_executable(&args.filename) || (extension == "cpp" || extension == "cc" || extension == "cxx" || extension == "c") {
		match compile_cpp(Path::new(&args.filename).to_path_buf(), &tempdir, args.compile_timeout, &args.compile_command) {
			Ok(result) => { executable = result }
			Err(error) => {
				println!("{}", "Compilation failed with the following errors:".red());
				println!("{}", error);
				return;
			}
		}
	}
	else {
		executable = tempdir.path().join(format!("{}.o", Path::new(&args.filename).file_name().expect("The provided filename is invalid!").to_str().expect("The provided filename is invalid!"))).to_str().expect("The provided filename is invalid!").to_string();
		fs::copy(&args.filename, &executable).expect("The provided filename is invalid!");

		let child = Command::new(&executable).spawn();
		if child.is_err() {
			println!("{}", "The provided file can't be executed!".red());
			return;
		}
	}

	// Progress bar styling
	let style: ProgressStyle = ProgressStyle::with_template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({eta})\n{correct} {incorrect} {ctrlc}")
		.expect("Progress bar creation failed!")
		.with_key("eta", |state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{:.1}s", state.eta().as_secs_f64()).expect("Displaying the progress bar failed!"))
		.progress_chars("#>-")
		.with_key("correct", |_state: &ProgressState, w: &mut dyn FmtWrite|
			write!(w, "{}", format!("{} {}", &SUCCESS_COUNT.get(), if GENERATE.load(Acquire) { "successful" } else { if SUCCESS_COUNT.get() != 1 { "correct answers" } else { "correct answer" } }).green()).expect("Displaying the progress bar failed!")
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
			None => false,
			Some(ext) => ".".to_owned() + &ext.to_str().unwrap_or("") == args.in_ext
		};
	});
	TEST_COUNT.store(input_files.len(), Release);

	if input_files.is_empty() {
		println!("{}", "There are no files in the input directory with the provided file extension".red());
		return;
	}

	// Testing for sio2jail errors before testing starts
	if sio2jail {
		let true_command_location = which("true");
		if true_command_location.is_err() {
			println!("{}", "The executable for the \"true\" command could not be found".red());
			return;
		}

		let test_input = tempdir.path().join(format!("test.in")).to_str().expect("The provided filename is invalid!").to_string();
		let test_input_path = Path::new(&test_input);
		File::create(test_input_path).expect("Failed to create temporary file!");

		let random_input_file_entry = input_files.get(0).expect("Couldn't get random input file").as_ref().expect("Failed to acquire reference!");
		let random_test_name = random_input_file_entry.path().file_stem().expect("Couldn't get the name of a random input file").to_str().expect("Couldn't get the name of a random input file").to_string();

		let (test_result, _) = run_test(&true_command_location.unwrap().to_str().expect("The executable for the \"true\" command has an invalid path").to_string(), test_input_path, &output_dir, &random_test_name, &args.out_ext, &(1 as u64), true, 0);
		match test_result {
			Error { error: ExecutionError::Sio2jailError(error), .. } => {
				if error == "Exception occurred: System error occured: perf event open failed: Permission denied: error 13: Permission denied\n" {
					println!("{}", "You need to run the following command to use toster with sio2jail. You may also put this option in your /etc/sysctl.conf. This will make the setting persist across reboots.".red());
					println!("{}", "sudo sysctl -w kernel.perf_event_paranoid=-1".bright_black().italic());
				}
				else {
					println!("Sio2jail error: {}", error.red());
				}

				return;
			}
			_ => {}
		}
	}

	TIME_BEFORE_TESTING.set(Instant::now()).expect("Couldn't store timestamp before testing!");
	// Running tests / generating output
	input_files.par_iter().progress_with_style(style).for_each(|input| {
		let input_file_entry = input.as_ref().expect("Failed to acquire reference!");
		let input_file_path = input_file_entry.path();
		let input_file_path_str = input_file_path.to_string_lossy().to_string();
		let test_name = input_file_entry.path().file_stem().expect(&*format!("The input file {} is invalid!", input_file_path_str)).to_str().expect(&*format!("The input file {} is invalid!", input_file_path_str)).to_string();

		let mut test_time: f64 = f64::MAX;
		let mut test_memory: i64 = -1;
		if args.generate {
			let input_file = File::open(input_file_path).expect(&*format!("Could not open input file {}", input_file_path_str));
			let output_file_path = format!("{}/{}{}", &output_dir, test_name, args.out_ext);
			let output_file = File::create(Path::new(&output_file_path)).expect("Failed to create output file!");

			let (execution_result, execution_error) = generate_output_default(&executable, input_file, output_file, &args.timeout);
			if !RECEIVED_CTRL_C.load(Acquire) {
				match execution_error {
					Ok(()) => {
						SUCCESS_COUNT.inc();
					}
					Err(error) => {
						match error {
							ExecutionError::TimedOut => { TIMED_OUT_COUNT.inc(); }
							ExecutionError::InvalidOutput => { INVALID_OUTPUT_COUNT.inc(); }
							ExecutionError::MemoryLimitExceeded => { MEMORY_LIMIT_EXCEEDED_COUNT.inc(); }
							ExecutionError::RuntimeError(_) => { RUNTIME_ERROR_COUNT.inc(); }
							ExecutionError::Sio2jailError(_) => { SIO2JAIL_ERROR_COUNT.inc(); }
						}
						let clone = Arc::clone(&ERRORS);
						clone.lock().expect("Failed to acquire mutex!").push(Error { test_name: test_name.clone(), error });
					}
				}

				test_time = execution_result.time_seconds;
			}
			else {
				thread::sleep(Duration::from_secs(u64::MAX));
			}
		}
		else {
			let (test_result, execution_result) = run_test(&executable, input_file_path.as_path(), &output_dir, &test_name, &args.out_ext, &args.timeout, sio2jail, memory_limit);
			test_time = execution_result.time_seconds;
			test_memory = execution_result.memory_kilobytes.unwrap_or(-1);

			if !RECEIVED_CTRL_C.load(Acquire) {
				match test_result {
					Correct { .. } => { SUCCESS_COUNT.inc(); }
					Incorrect { .. } => { INCORRECT_COUNT.inc(); }
					Error { error: ExecutionError::TimedOut, .. } => { TIMED_OUT_COUNT.inc(); }
					Error { error: ExecutionError::InvalidOutput, .. } => { INVALID_OUTPUT_COUNT.inc(); }
					Error { error: ExecutionError::MemoryLimitExceeded, .. } => { MEMORY_LIMIT_EXCEEDED_COUNT.inc(); }
					Error { error: ExecutionError::RuntimeError(_), .. } => { RUNTIME_ERROR_COUNT.inc(); }
					Error { error: ExecutionError::Sio2jailError(_), .. } => { SIO2JAIL_ERROR_COUNT.inc(); }
					NoOutputFile { .. } => { NO_OUTPUT_FILE_COUNT.inc(); }
				}

				if !test_result.is_correct() {
					let clone = Arc::clone(&ERRORS);
					clone.lock().expect("Failed to acquire mutex!").push(test_result);
				}
			}
			else {
				thread::sleep(Duration::from_secs(u64::MAX));
			}
		}

		if !RECEIVED_CTRL_C.load(Acquire) {
			let slowest_test_clone = Arc::clone(&SLOWEST_TEST);
			let mut slowest_test_mutex = slowest_test_clone.lock().expect("Failed to acquire mutex!");
			if test_time > slowest_test_mutex.0 {
				*slowest_test_mutex = (test_time, test_name.clone());
			}

			let most_memory_clone = Arc::clone(&MOST_MEMORY_USED);
			let mut most_memory_mutex = most_memory_clone.lock().expect("Failed to acquire mutex!");
			if test_memory > most_memory_mutex.0 {
				*most_memory_mutex = (test_memory, test_name.clone());
			}
		}
		else {
			thread::sleep(Duration::from_secs(u64::MAX));
		}
	});

	print_output(false)
}
