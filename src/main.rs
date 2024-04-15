mod args;
mod test_errors;
mod testing_utils;
mod prepare_input;
mod executor;
mod generic_utils;
mod test_summary;
mod temp_files;
mod checker;
mod compiler;
mod formatted_error;
mod output;

use std::{fs, panic};
use std::fs::File;
use std::panic::PanicInfo;
use std::path::PathBuf;
use std::process::{exit, ExitCode};
use std::sync::{Arc, Mutex};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::{Acquire, Release};
use clap::Parser;
use colored::Colorize;
use human_panic::{handle_dump, print_msg};
use indicatif::{ParallelProgressIterator, ProgressBar};
use rayon::prelude::*;
use tempfile::tempdir;
use args::Args;
use crate::args::{ActionType, InputConfig, ParsedConfig};
use crate::args::ExecuteMode::*;
use crate::checker::Checker;
use crate::compiler::Compiler;
use crate::executor::simple::SimpleExecutor;
use crate::prepare_input::{prepare_file_inputs, Test, TestingInputs};
use crate::executor::{AnyTestExecutor, test_to_temp, TestExecutor};
use crate::test_errors::{ExecutionMetrics, TestError};
use crate::test_errors::TestError::{Cancelled, ProgramError};
use crate::test_summary::TestSummary;
use crate::testing_utils::compare_output;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use crate::executor::sio2jail::Sio2jailExecutor;
use crate::formatted_error::FormattedError;
use crate::generic_utils::halt;
use crate::output::{get_progress_bar, print_output, start_initial_spinner};

static RECEIVED_CTRL_C: AtomicBool = AtomicBool::new(false);

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

fn check_ctrlc() -> Result<(), TestError> {
	if RECEIVED_CTRL_C.load(Acquire) { Err(Cancelled) }
	else { Ok(()) }
}

fn init_runner(executable: PathBuf, config: &ParsedConfig) -> Result<AnyTestExecutor, FormattedError> {
	Ok(match config.execute_mode {
		Simple => AnyTestExecutor::Simple(SimpleExecutor {
			executable_path: executable,
			timeout: config.execute_timeout,
		}),
		#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
		Sio2jail { memory_limit } => AnyTestExecutor::Sio2Jail(Sio2jailExecutor::init_and_test(
			config.execute_timeout,
			executable,
			memory_limit,
		)?),
	})
}

fn map_tests<T>(
	inputs: TestingInputs<T>,
	progress_bar: ProgressBar,
	test_summary: &Arc<Mutex<Option<TestSummary>>>,
	callback: impl Fn(Test) -> Result<ExecutionMetrics, TestError> + Sync
) where T: IndexedParallelIterator<Item = Test> {
	inputs.iterator.progress_with(progress_bar).try_for_each(|input| {
		let test_name = input.test_name.clone();

		let result = callback(input);

		let mut test_summary = test_summary.lock().expect("Failed to lock test summary mutex");
		let test_summary = test_summary.as_mut().unwrap();
		match result {
			Ok(metrics) => test_summary.add_success(&metrics, &test_name),
			Err(Cancelled) => return None,
			Err(error) => test_summary.add_test_error(error, test_name),
		};
		Some(())
	});
}

fn main() -> ExitCode {
	setup_panic();

	if let Err(error) = try_main() {
		println!("{}", error);
		return ExitCode::FAILURE;
	}
	ExitCode::SUCCESS
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
			exit(0);
		}).expect("Error setting Ctrl-C handler");
	}

	let tempdir = tempdir().expect("Failed to create temporary directory");

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

	let (runner, checker, inputs) = start_initial_spinner(|spinner| {
		let inputs_handle = spinner.add_job("preparing inputs", || {
			match &config.input {
				InputConfig::Directory { directory, ext } => {
					prepare_file_inputs(directory, ext)
				},
			}
		});

		let runner_handle = spinner.add_job("compiling program", || {
			let (executable, _) = compiler
				.prepare_executable(&config.source_path, "program")
				.map_err(|error| error.to_formatted(false))?;
			// TODO: Restore or remove
			// println! doesn't work correctly with the progress bar

			// if let Some(compilation_time) = compilation_time {
			// 	println!("{}", format!("Program compilation completed in {:.2}", compilation_time.as_secs_f32()).green());
			// }
			init_runner(executable, &config)
		});

		let checker_handle = if let ActionType::Checker { path } = &config.action_type {
			Some(spinner.add_job("compiling checker", || {
				let (executable, _) = compiler
					.prepare_executable(path, "checker")
					.map_err(|error| error.to_formatted(true))?;
				// TODO: Restore or remove
				// if let Some(compilation_time) = compilation_time {
				// 	println!("{}", format!("Checker compilation completed in {:.2}", compilation_time.as_secs_f32()).green());
				// }
				Ok(Checker::new(executable, config.execute_timeout))
			}))
		} else { None };

		Ok((
			runner_handle.join().unwrap(),
			checker_handle.map(|handle| handle.join().unwrap()),
			inputs_handle.join().unwrap(),
		))
	})?;

	*test_summary.lock().expect("Failed to lock test summary mutex") = Some(TestSummary::new(config.generate_mode(), inputs.test_count));
	let progress_bar = get_progress_bar(&test_summary);

	match config.action_type {
		ActionType::Generate { output_directory, output_ext } => {
			map_tests(inputs, progress_bar, &test_summary, |input| {
				check_ctrlc()?;

				let output_file_path = output_directory.join(format!("{}{}", input.test_name, &output_ext));
				let file = File::create(output_file_path).expect("Failed to create output file");
				check_ctrlc()?;

				let (metrics, result) = runner.test_to_file(&input.input_source.get_file(), &file);
				check_ctrlc()?;

				result.map_err(|error| ProgramError { error })?;
				Ok(metrics)
			});
		},
		ActionType::SimpleCompare { output_directory, output_ext } => {
			map_tests(inputs, progress_bar, &test_summary, |input| {
				check_ctrlc()?;

				let (metrics, result) = test_to_temp(&runner, &input.input_source.get_file());
				check_ctrlc()?;

				let result = result.map_err(|error| ProgramError { error })?;
				let output_file_path = output_directory.join(format!("{}{}", input.test_name, output_ext));
				compare_output(&output_file_path, result)?;
				check_ctrlc()?;

				Ok(metrics)
			});
		},
		ActionType::Checker { .. } => {
			let checker = checker.expect("Checker should be initialized");
			map_tests(inputs, progress_bar, &test_summary, |input| {
				check_ctrlc()?;

				let checker_input = Checker::prepare_checker_input(&input.input_source);
				check_ctrlc()?;

				let (metrics, result) = runner.test_to_file(
					&input.input_source.get_file(),
					&checker_input,
				);
				check_ctrlc()?;

				result.map_err(|error| ProgramError { error })?;
				checker.check(checker_input)?;
				check_ctrlc()?;

				Ok(metrics)
			})
		}
	}

	print_output(false, &mut test_summary.lock().expect("Failed to lock test summary mutex"));
	Ok(())
}
