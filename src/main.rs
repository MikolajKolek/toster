mod args;
mod test_errors;
mod testing_utils;
mod prepare_input;
mod executor;
mod generic_utils;
mod test_summary;
mod pipes;

use std::{fs, panic, thread};
use std::ffi::OsStr;
use std::fmt::Write as FmtWrite;
use std::panic::PanicInfo;
use std::path::PathBuf;
use std::process::{Command, exit};
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::atomic::Ordering::{Acquire, Release};
use std::time::{Duration, Instant};
use clap::Parser;
use colored::Colorize;
use human_panic::{handle_dump, print_msg};
use indicatif::{ParallelProgressIterator, ProgressBar, ProgressState, ProgressStyle};
use is_executable::is_executable;
use rayon::prelude::*;
use tempfile::tempdir;
use args::Args;
use crate::args::ActionType::{Checker, Generate};
use crate::args::{ActionType, InputConfig, ParsedConfig};
use crate::args::ExecuteMode::*;
use crate::executor::simple::SimpleExecutor;
use crate::executor::sio2jail::Sio2jailExecutor;
use crate::prepare_input::prepare_file_inputs;
use crate::executor::TestExecutor;
use crate::test_errors::TestError::ProgramError;
use crate::test_summary::TestSummary;
use crate::testing_utils::{compare_output, compile_cpp};

static TIME_BEFORE_TESTING: OnceLock<Instant> = OnceLock::new();
static TEST_COUNT: AtomicUsize = AtomicUsize::new(0);
static GENERATE: AtomicBool = AtomicBool::new(false);

static RECEIVED_CTRL_C: AtomicBool = AtomicBool::new(false);
static PANICKING: AtomicBool = AtomicBool::new(false);

fn print_output(stopped_early: bool, test_summary: &mut TestSummary) {
	if TIME_BEFORE_TESTING.get().is_none() {
		println!("{}", "Toster was stopped before testing could start".red());
		exit(0);
	}

	let testing_time = TIME_BEFORE_TESTING.get().unwrap().elapsed().as_secs_f64();
	let not_tested_count = &TEST_COUNT.load(Acquire) - test_summary.total;

	if stopped_early {
		println!();
	}

	let mut additional_info = String::new();
	if let Some(slowest_test) = &test_summary.slowest_test {
		additional_info = format!(
			" (Slowest test: {} at {:.3}s)",
			slowest_test.1,
		  	slowest_test.0.as_secs_f32(),
		)
	}
	if let Some(most_memory) = &test_summary.most_memory_used {
		if !additional_info.is_empty() { additional_info += ", " }
		additional_info += &format!("most memory used: {} at {}KiB", most_memory.1, most_memory.0);
	}

	println!(
		"{} {} {:.2}s{}\nResults: {}",
        if GENERATE.load(Acquire) { "Generating" } else { "Testing" },
        if stopped_early {"stopped after"} else {"finished in"},
        testing_time,
        additional_info,
        test_summary.format_counts(Some(not_tested_count)),
	);

	let incorrect_results=  test_summary.get_errors();
	if !incorrect_results.is_empty() {
		println!("Errors were found in the following tests:");

		for (test_name, error) in incorrect_results.iter() {
			println!("{}", error.to_string(test_name));
		}
	}

	exit(0);
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
					exit(0);
				}
				else {
					thread::sleep(Duration::from_secs(u64::MAX));
				}
			}));
		}
	}
}

#[must_use]
fn check_ctrlc() -> Option<()> {
	if RECEIVED_CTRL_C.load(Acquire) { None }
	else { Some(()) }
}

fn main() {
	setup_panic();

    let config = match ParsedConfig::try_from(Args::parse()) {
		Ok(config) => config,
		Err(error) => {
			println!("{}", error.red());
			exit(1);
		},
	};

    GENERATE.store(config.generate_mode(), Release);

	let test_summary = Arc::new(Mutex::new(
		TestSummary::new(config.generate_mode())
	));
	{
		let test_summary = test_summary.clone();
		ctrlc::set_handler(move || {
			RECEIVED_CTRL_C.store(true, Release);
			print_output(true, &mut test_summary.lock().expect("Failed to lock test summary mutex"));
		}).expect("Error setting Ctrl-C handler");
	}

	let tempdir = tempdir().expect("Failed to create temporary directory!");

	if let Generate { output_directory, .. } = &config.action_type {
		if !output_directory.is_dir() {
			fs::create_dir_all(output_directory).expect("Failed to create output directory");
		}
	}

	// Compiling
	let extension = config.source_path.extension().unwrap_or(OsStr::new("")).to_str().expect("Couldn't get the extension of the provided file");
	let executable: PathBuf = if !is_executable(&config.source_path) || (extension == "cpp" || extension == "cc" || extension == "cxx" || extension == "c") {
		match compile_cpp(&config.source_path, &tempdir, config.compile_timeout, &config.compile_command) {
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
		let executable = tempdir.path().join(format!("{}.o", config.source_path.file_name().expect("The provided filename is invalid!").to_str().expect("The provided filename is invalid!")));
		fs::copy(&config.source_path, &executable).expect("The provided filename is invalid!");

		let Ok(mut child) = Command::new(&executable).spawn() else {
			println!("{}", "The provided file can't be executed!".red());
			return;
		};
		child.kill().unwrap_or(());
		executable
	};

	// Checker compiling
	let checker_executable: Option<PathBuf> = if let Checker { path } = &config.action_type {
		let checker_extension = path.extension().unwrap_or(OsStr::new("")).to_str().expect("Couldn't get the extension of the provided file");

		if !is_executable(&path) || (checker_extension == "cpp" || checker_extension == "cc" || checker_extension == "cxx" || checker_extension == "c") {
			match compile_cpp(&path, &tempdir, config.compile_timeout, &config.compile_command) {
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
			let checker_executable = tempdir.path().join(format!("{}.o", path.file_name().expect("The provided checker is invalid!").to_str().expect("The provided checker is invalid!")));
			fs::copy(&path, &checker_executable).expect("The provided filename is invalid!");

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

	let runner: Box<dyn TestExecutor> = match config.execute_mode {
		Simple => Box::new(SimpleExecutor {
			executable_path: executable,
			timeout: config.execute_timeout,
		}),
		#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
		Sio2jail { memory_limit } => {
			let runner = Sio2jailExecutor::init_and_test(
				config.execute_timeout,
				executable,
				memory_limit,
			);
			match runner {
				Ok(runner) => Box::new(runner),
				Err(error) => {
					println!("{}", error.red());
					exit(1);
				}
			}
		},
	};

	// Progress bar styling
    let style: ProgressStyle = {
        let test_summary = test_summary.clone();
        ProgressStyle::with_template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({eta})\n{counts} {ctrlc}")
            .expect("Progress bar creation failed!")
            .with_key("eta", |state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{:.1}s", state.eta().as_secs_f64()).expect("Displaying the progress bar failed!"))
            .progress_chars("#>-")
            .with_key("counts", move |_state: &ProgressState, w: &mut dyn FmtWrite| {
                write!(w, "{}", test_summary.lock().expect("Failed to lock test summary mutex").format_counts(None)).expect("Displaying the progress bar failed!")
            })
            .with_key("ctrlc", |_state: &ProgressState, w: &mut dyn FmtWrite|
                write!(w, "{}", "(Press Ctrl+C to stop testing and print current results)".bright_black()).expect("Displaying the progress bar Ctrl+C message failed!")
            )
    };

	let inputs = match config.input {
		InputConfig::Directory { directory, ext } => {
			prepare_file_inputs(&directory, &ext)
		},
	};
	TEST_COUNT.store(inputs.test_count, Release);

	// Running tests / generating output
	TIME_BEFORE_TESTING.set(Instant::now()).expect("Couldn't store timestamp before testing!");
	let progress_bar = ProgressBar::new(inputs.test_count as u64).with_style(style);
	inputs.iterator.progress_with(progress_bar).try_for_each(|input| -> Option<()> {
		debug_assert!(!RECEIVED_CTRL_C.load(Acquire), "Test started after CTRL+C has been pressed");

		let (metrics, output) = runner.test_to_string(&input.input_source);
		check_ctrlc()?;

		let output = match output {
			Ok(output) => output,
			Err(error) => {
				test_summary.lock().expect("Failed to lock test summary mutex")
					.add_test_error(ProgramError { error }, input.test_name);
				return Some(());
			}
		};

		check_ctrlc()?;

		match &config.action_type {
			Generate { output_directory, output_ext } => {
				let output_file_path = output_directory.join(format!("{}{}", input.test_name, output_ext));
				fs::write(&output_file_path, &output).expect("Failed to save test output");
			},
			ActionType::SimpleCompare { output_directory, output_ext } => {
				let output_file_path = output_directory.join(format!("{}{}", input.test_name, output_ext));
				if let Err(error) = compare_output(&output_file_path, &output) {
					test_summary.lock().expect("Failed to lock test summary mutex")
						.add_test_error(error, input.test_name);
					return Some(());
				}
			},
			Checker { .. } => {
				todo!("Checker support is not implemented in this version");
			},
		}

		test_summary.lock().expect("Failed to lock test summary mutex")
			.add_success(&metrics, &input.test_name);
		check_ctrlc()?;
		Some(())
	});

	print_output(false, &mut test_summary.lock().expect("Failed to lock test summary mutex"));
}
