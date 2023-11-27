mod args;
mod test_errors;
mod testing_utils;
mod prepare_input;
mod executor;
mod generic_utils;
mod test_summary;
mod pipes;
mod checker;
mod compiler;
mod formatted_error;

use std::{fs, panic};
use std::fmt::Write as FmtWrite;
use std::panic::PanicInfo;
use std::path::PathBuf;
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::{Acquire, Release};
use clap::Parser;
use colored::Colorize;
use human_panic::{handle_dump, print_msg};
use indicatif::{ParallelProgressIterator, ProgressBar, ProgressState, ProgressStyle};
use rayon::prelude::*;
use tempfile::tempdir;
use args::Args;
use crate::args::{ActionType, InputConfig, ParsedConfig};
use crate::args::ExecuteMode::*;
use crate::checker::Checker;
use crate::compiler::Compiler;
use crate::executor::simple::SimpleExecutor;
use crate::prepare_input::prepare_file_inputs;
use crate::executor::TestExecutor;
use crate::test_errors::TestError::ProgramError;
use crate::test_summary::TestSummary;
use crate::testing_utils::compare_output;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use crate::executor::sio2jail::Sio2jailExecutor;
use crate::formatted_error::FormattedError;
use crate::generic_utils::halt;

static RECEIVED_CTRL_C: AtomicBool = AtomicBool::new(false);

fn print_output(stopped_early: bool, test_summary: &mut Option<TestSummary>) {
	let Some(test_summary) = test_summary else {
		println!("{}", "Toster was stopped before testing could start".red());
		exit(0);
	};

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
        if test_summary.generate_mode { "Generating" } else { "Testing" },
        if stopped_early {"stopped after"} else {"finished in"},
        test_summary.start_time.elapsed().as_secs_f64(),
        additional_info,
        test_summary.format_counts(true),
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
	let is_panicking = AtomicBool::new(false);
	match human_panic::PanicStyle::default() {
		human_panic::PanicStyle::Debug => {}
		human_panic::PanicStyle::Human => {
			let meta = human_panic::metadata!();

			panic::set_hook(Box::new(move |info: &PanicInfo| {
				if is_panicking.load(Acquire) {
					halt();
				}
				is_panicking.store(true, Release);

				let file_path = handle_dump(&meta, info);
				print_msg(file_path, &meta).expect("human-panic: printing error message to console failed");
				exit(0);
			}));
		}
	}
}

#[must_use]
fn check_ctrlc() -> Option<()> {
	if RECEIVED_CTRL_C.load(Acquire) { None }
	else { Some(()) }
}

fn init_runner(executable: PathBuf, config: &ParsedConfig) -> Box<dyn TestExecutor> {
	match config.execute_mode {
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
	}
}

fn main() {
	setup_panic();

	if let Err(error) = try_main() {
		println!("{}", error);
		exit(1);
	}
}

fn try_main() -> Result<(), FormattedError> {
    let config = ParsedConfig::try_from(Args::parse())
		.map_err(|error| FormattedError::from_str(&error))?;
	let test_summary: Arc<Mutex<Option<TestSummary>>> = Arc::new(Mutex::new(None));
	{
		let test_summary = test_summary.clone();
		ctrlc::set_handler(move || {
			RECEIVED_CTRL_C.store(true, Release);
			print_output(true, &mut test_summary.lock().expect("Failed to lock test summary mutex"));
		}).expect("Error setting Ctrl-C handler");
	}

	let tempdir = tempdir().expect("Failed to create temporary directory!");

	if let ActionType::Generate { output_directory, .. } = &config.action_type {
		if !output_directory.is_dir() {
			fs::create_dir_all(output_directory).expect("Failed to create output directory");
		}
	}

	let compiler = Compiler {
		tempdir: &tempdir,
		compile_timeout: config.compile_timeout,
		compile_command: &config.compile_command,
	};

	let executable = {
		let (executable, compilation_time) = compiler
			.prepare_executable(&config.source_path, "program")
			.map_err(|error| error.to_formatted(false))?;
		if let Some(compilation_time) = compilation_time {
			println!("{}", format!("Program compilation completed in {:.2}", compilation_time.as_secs_f32()).green());
		}
		executable
	};

	let checker_executable = if let ActionType::Checker { path } = &config.action_type {
		let (executable, compilation_time) = compiler
			.prepare_executable(path, "checker")
			.map_err(|error| error.to_formatted(true))?;
		if let Some(compilation_time) = compilation_time {
			println!("{}", format!("Checker compilation completed in {:.2}", compilation_time.as_secs_f32()).green());
		}
		Some(executable)
	} else { None };

	let runner = init_runner(executable, &config);

	let checker = checker_executable.map(|checker_executable| {
		Checker::new(checker_executable, config.execute_timeout)
	});

	// Progress bar styling
    let style: ProgressStyle = {
        let test_summary = test_summary.clone();
        ProgressStyle::with_template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({eta})\n{counts} {ctrlc}")
            .expect("Progress bar creation failed!")
            .with_key("eta", |state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{:.1}s", state.eta().as_secs_f64()).expect("Displaying the progress bar failed!"))
            .progress_chars("#>-")
            .with_key("counts", move |_state: &ProgressState, w: &mut dyn FmtWrite| {
                write!(w, "{}", test_summary.lock().expect("Failed to lock test summary mutex").as_ref().unwrap().format_counts(false)).expect("Displaying the progress bar failed!")
            })
            .with_key("ctrlc", |_state: &ProgressState, w: &mut dyn FmtWrite|
                write!(w, "{}", "(Press Ctrl+C to stop testing and print current results)".bright_black()).expect("Displaying the progress bar Ctrl+C message failed!")
            )
    };

	let inputs = match &config.input {
		InputConfig::Directory { directory, ext } => {
			prepare_file_inputs(directory, ext)
		},
	};
	*test_summary.lock().expect("Failed to lock test summary mutex") = Some(TestSummary::new(config.generate_mode(), inputs.test_count));

	let progress_bar = ProgressBar::new(inputs.test_count as u64).with_style(style);
	inputs.iterator.progress_with(progress_bar).try_for_each(|input| -> Option<()> {
		check_ctrlc()?;

		let (metrics, output) = runner.test_to_string(input.input_source.get_stdin());
		check_ctrlc()?;

		let output = match output {
			Ok(output) => output,
			Err(error) => {
				test_summary.lock().expect("Failed to lock test summary mutex").as_mut().unwrap()
					.add_test_error(ProgramError { error }, input.test_name);
				return Some(());
			}
		};

		check_ctrlc()?;

		match &config.action_type {
			ActionType::Generate { output_directory, output_ext } => {
				let output_file_path = output_directory.join(format!("{}{}", input.test_name, output_ext));
				fs::write(&output_file_path, &output).expect("Failed to save test output");
			},
			ActionType::SimpleCompare { output_directory, output_ext } => {
				let output_file_path = output_directory.join(format!("{}{}", input.test_name, output_ext));
				if let Err(error) = compare_output(&output_file_path, &output) {
					test_summary.lock().expect("Failed to lock test summary mutex").as_mut().unwrap()
						.add_test_error(error, input.test_name);
					return Some(());
				}
			},
			_ => {},
		}
		if let Some(checker) = &checker {
			if let Err(error) = checker.check(&input.input_source, &output) {
				test_summary.lock().expect("Failed to lock test summary mutex").as_mut().unwrap()
					.add_test_error(error, input.test_name);
				return Some(());
			}
		}

		test_summary.lock().expect("Failed to lock test summary mutex").as_mut().unwrap()
			.add_success(&metrics, &input.test_name);
		check_ctrlc()?;
		Some(())
	});

	print_output(false, &mut test_summary.lock().expect("Failed to lock test summary mutex"));
	Ok(())
}
