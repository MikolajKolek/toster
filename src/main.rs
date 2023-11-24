mod args;
mod test_result;
mod testing_utils;
mod prepare_input;
mod run;
mod generic_utils;
mod test_summary;

use std::{fs, panic, process, thread};
use std::ffi::OsStr;
use std::fmt::Write as FmtWrite;
use std::fs::File;
use std::io::{Read, Write};
use std::panic::PanicInfo;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::atomic::Ordering::{Acquire, Release};
use std::time::{Duration, Instant};
use clap::Parser;
use colored::Colorize;
use human_panic::{handle_dump, print_msg};
use indicatif::{ParallelProgressIterator, ProgressBar, ProgressState, ProgressStyle};
use is_executable::is_executable;
use lazy_static::lazy_static;
use rayon::prelude::*;
use tempfile::tempdir;
use which::which;
use args::Args;
use crate::generic_utils::OptionExt;
use crate::prepare_input::prepare_file_inputs;
use crate::run::BasicTestRunner;
use crate::test_result::TestResult;
use crate::test_summary::TestSummary;
use crate::testing_utils::{compile_cpp, fill_tempfile_pool, init_sio2jail};
use crate::TestResult::{Correct, ProgramError, Incorrect};

lazy_static! {
    static ref SLOWEST_TEST: Arc<Mutex<Option<(Duration, String)>>> = Arc::new(Mutex::new(None));
    static ref MOST_MEMORY_USED: Arc<Mutex<Option<(i64, String)>>> = Arc::new(Mutex::new(None));
}

static TIME_BEFORE_TESTING: OnceLock<Instant> = OnceLock::new();
static TEST_COUNT: AtomicUsize = AtomicUsize::new(0);
static GENERATE: AtomicBool = AtomicBool::new(false);

static RECEIVED_CTRL_C: AtomicBool = AtomicBool::new(false);
static PANICKING: AtomicBool = AtomicBool::new(false);

fn print_output(stopped_early: bool, test_summary: &mut TestSummary) {
	// let slowest_test_clone = Arc::clone(&SLOWEST_TEST);
	// let most_memory_clone = Arc::clone(&MOST_MEMORY_USED);
	// let slowest_test_mutex = slowest_test_clone.lock().expect("Failed to acquire mutex!");
	// let most_memory_mutex = most_memory_clone.lock().expect("Failed to acquire mutex!");

	if TIME_BEFORE_TESTING.get().is_none() {
		println!("{}", "Toster was stopped before testing could start".red());
		process::exit(0);
	}

	let testing_time = TIME_BEFORE_TESTING.get().unwrap().elapsed().as_secs_f64();
	let not_tested_count = &TEST_COUNT.load(Acquire) - test_summary.total;

	if stopped_early {
		println!();
	}

	let mut additional_info = String::new();
	// if slowest_test_mutex.0 != -1f64 {
	// 	additional_info = format!(" (Slowest test: {} at {:.3}s{})",
	// 	                          slowest_test_mutex.1,
	// 	                          slowest_test_mutex.0,
	// 	                          if most_memory_mutex.0 != -1 { format!(", most memory used: {} at {}KiB", most_memory_mutex.1, most_memory_mutex.0) } else { String::new() }
	// 	)
	// }

	println!(
		"{} {} {:.2}s{}\nResults: {}",
        if GENERATE.load(Acquire) { "Generating" } else { "Testing" },
        if stopped_early {"stopped after"} else {"finished in"},
        testing_time,
        additional_info,
        test_summary.format_counts(Some(not_tested_count)),
	);

	let incorrect_results=  test_summary.get_incorrect_results();
	if !incorrect_results.is_empty() {
		println!("Errors were found in the following tests:");

		for test_error in incorrect_results.iter() {
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

#[warn(must_use)]
fn check_ctrlc() -> Option<()> {
	if RECEIVED_CTRL_C.load(Acquire) { None }
	else { Some(()) }
}

fn main() {
	setup_panic();

    let args = Args::parse();
    GENERATE.store(args.generate, Release);
    let input_dir = args.io.as_ref().unwrap_or(&args.r#in);
    let output_dir = args.io.as_ref().unwrap_or(&args.out);

	let mut test_summary = Arc::new(Mutex::new(TestSummary::new(args.generate)));

	let test_summary_1 = test_summary.clone();
	ctrlc::set_handler(move || {
		RECEIVED_CTRL_C.store(true, Release);
		print_output(true, &mut test_summary_1.lock().expect("Failed to lock test summary mutex"));
	}).expect("Error setting Ctrl-C handler");

	let tempdir = tempdir().expect("Failed to create temporary directory!");
	fill_tempfile_pool(&tempdir);

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
	if args.checker.is_some() && args.generate {
		println!("{}", "You can't have the --generate and --checker flags on at the same time.".red());
		return;
	}
	if sio2jail && memory_limit == 0 {
		memory_limit = 1024 * 1024;
	}
	if sio2jail && !init_sio2jail() {
		return;
	}

	// Making sure that the input and output directories as well as the source code file exist
	if args.io.as_ref().is_some_and(|io| !io.is_dir()) {
		println!("{}", "The input/output directory does not exist".red());
		return;
	}
	if !input_dir.is_dir() {
		println!("{}", "The input directory does not exist".red());
		return;
	}
	if !output_dir.is_dir() && args.checker.is_none() {
		if args.generate {
			fs::create_dir(output_dir).expect("Failed to create output directory!");
		}
		else {
			println!("{}", "The output directory does not exist".red());
			return;
		}
	}
	if !args.filename.is_file() {
		println!("{}", "The provided file does not exist".red());
		return;
	}
	if args.checker.as_ref().is_some_and(|checker| !checker.is_file()) {
		println!("{}", "The provided checker file does not exist".red());
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
	let extension = args.filename.extension().unwrap_or(OsStr::new("")).to_str().expect("Couldn't get the extension of the provided file");
	let executable: PathBuf = if !is_executable(&args.filename) || (extension == "cpp" || extension == "cc" || extension == "cxx" || extension == "c") {
		match compile_cpp(&args.filename, &tempdir, args.compile_timeout, &args.compile_command) {
			Ok((compiled_executable, compilation_time)) => {
				println!("{}", format!("Compilation completed in {:.2}s", compilation_time).green());
				compiled_executable
			}
			Err(error) => {
				println!("{}", "Compilation failed with the following errors:".red());
				println!("{}", error);
				return;
			}
		}
	}
	else {
		let executable = tempdir.path().join(format!("{}.o", args.filename.file_name().expect("The provided filename is invalid!").to_str().expect("The provided filename is invalid!")));
		fs::copy(&args.filename, &executable).expect("The provided filename is invalid!");

		let Ok(mut child) = Command::new(&executable).spawn() else {
			println!("{}", "The provided file can't be executed!".red());
			return;
		};
		child.kill().unwrap_or(());
		executable
	};

	// Checker compiling
	let checker_executable: Option<PathBuf> = if let Some(checker_path) = args.checker {
		let checker_extension = checker_path.extension().unwrap_or(OsStr::new("")).to_str().expect("Couldn't get the extension of the provided file");

		if !is_executable(&checker_path) || (checker_extension == "cpp" || checker_extension == "cc" || checker_extension == "cxx" || checker_extension == "c") {
			match compile_cpp(&checker_path, &tempdir, args.compile_timeout, &args.compile_command) {
				Ok((compiled_executable, compilation_time)) => {
					println!("{}", format!("Checker compilation completed in {:.2}s", compilation_time).green());
					Some(compiled_executable)
				}
				Err(error) => {
					println!("{}", "Checker compilation failed with the following errors:".red());
					println!("{}", error);
					return;
				}
			}
		}
		else {
			let checker_executable = tempdir.path().join(format!("{}.o", checker_path.file_name().expect("The provided checker is invalid!").to_str().expect("The provided checker is invalid!")));
			fs::copy(&checker_path, &checker_executable).expect("The provided filename is invalid!");

			let Ok(mut child) = Command::new(&checker_executable).spawn() else {
				println!("{}", "The provided checker can't be executed!".red());
				return;
			};
			child.kill().unwrap_or(());

			Some(checker_executable)
		}
	} else {
		None
	};

	// Progress bar styling
	let test_summary_2 = test_summary.clone();
	let style: ProgressStyle = ProgressStyle::with_template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({eta})\n{counts} {ctrlc}")
		.expect("Progress bar creation failed!")
		.with_key("eta", |state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{:.1}s", state.eta().as_secs_f64()).expect("Displaying the progress bar failed!"))
		.progress_chars("#>-")
		.with_key("counts", move |_state: &ProgressState, w: &mut dyn FmtWrite| {
            write!(w, "{}", test_summary_2.lock().expect("Failed to lock test summary mutex").format_counts(None)).expect("Displaying the progress bar failed!")
		})
		.with_key("ctrlc", |_state: &ProgressState, w: &mut dyn FmtWrite|
			write!(w, "{}", "(Press Ctrl+C to stop testing and print current results)".bright_black()).expect("Displaying the progress bar Ctrl+C message failed!")
		);

	let inputs = prepare_file_inputs(&input_dir, &args.in_ext);
	TEST_COUNT.store(inputs.test_count, Release);

	// Testing for sio2jail errors before testing starts
	if sio2jail {
		let Ok(true_command_location) = which("true") else {
			println!("{}", "The executable for the \"true\" command could not be found".red());
			return;
		};

		let test_input_path = tempdir.path().join("test.in");
		File::create(&test_input_path).expect("Failed to create temporary file!");

		todo!("File iterator makes it impossible to access the first input file");
		// let random_input_file_entry = input_files.get(0).expect("Couldn't get random input file").as_ref().expect("Failed to acquire reference!");
		// let random_test_name = random_input_file_entry.path().file_stem().expect("Couldn't get the name of a random input file").to_str().expect("Couldn't get the name of a random input file").to_string();
		//
		// let (test_result, _) = run_test(&true_command_location, None, &test_input_path, &output_dir, &random_test_name, &args.out_ext, &( 1u64), true, 0);
		// if let ProgramError { error: ExecutionError::Sio2jailError(error), .. } = test_result {
		// 	if error == "Exception occurred: System error occured: perf event open failed: Permission denied: error 13: Permission denied\n" {
		// 		println!("{}", "You need to run the following command to use toster with sio2jail. You may also put this option in your /etc/sysctl.conf. This will make the setting persist across reboots.".red());
		// 		println!("{}", "sudo sysctl -w kernel.perf_event_paranoid=-1".bright_black().italic());
		// 	}
		// 	else {
		// 		println!("Sio2jail error: {}", error.red());
		// 	}
		//
		// 	return;
		// }
	}

	let runner = BasicTestRunner {
		executable_path: executable,
		timeout: Duration::from_secs(args.timeout)
	};

	// Running tests / generating output
	TIME_BEFORE_TESTING.set(Instant::now()).expect("Couldn't store timestamp before testing!");
	let progress_bar = ProgressBar::new(inputs.test_count as u64).with_style(style);
	inputs.iterator.progress_with(progress_bar).try_for_each(|input| -> Option<()> {
		debug_assert!(!RECEIVED_CTRL_C.load(Acquire), "Test started after CTRL+C has been pressed");

		let (metrics, output) = runner.test_to_vec(&input.input_source);

		check_ctrlc();

		let output = match output {
			Ok(output) => {
				test_summary.lock().expect("Failed to lock test summary mutex").increment_success();
				output
			}
			Err(error) => {
				let program_error = ProgramError { test_name: input.test_name.clone(), error };
				test_summary.lock().expect("Failed to lock test summary mutex").add_test_result(program_error);
				return Some(());
			}
		};

		let slowest_test_clone = Arc::clone(&SLOWEST_TEST);
		let mut slowest_test_mutex = slowest_test_clone.lock().expect("Failed to acquire mutex!");
		if slowest_test_mutex.is_none_or(|(time, _)| &metrics.time > time) {
			*slowest_test_mutex = Some((metrics.time, input.test_name.clone()));
		}

		if let Some(new_memory) = &metrics.memory_kilobytes {
			let most_memory_clone = Arc::clone(&MOST_MEMORY_USED);
			let mut most_memory_mutex = most_memory_clone.lock().expect("Failed to acquire mutex!");
			if most_memory_mutex.is_none_or(|(memory, _)| new_memory > memory) {
				*most_memory_mutex = Some((*new_memory, input.test_name.clone()));
			}
		}

		if args.generate {
			let output_file_path = output_dir.join(format!("{}{}", input.test_name, args.out_ext));
			let mut output_file = File::create(&output_file_path).expect("Failed to create output file!");
			output_file.write_all(&output).expect("Failed to save test output");

			check_ctrlc();

			return Some(())
		}

		// Test

		check_ctrlc();

		Some(())
	});

	print_output(false, &mut test_summary.lock().expect("Failed to lock test summary mutex"));
}
