use std::path::PathBuf;
use std::time::Duration;
use clap::Parser;
use crate::args::ExecuteMode::{Simple};

#[derive(Parser, Debug)]
#[command(name = "Toster", version, about = "A simple-as-toast tester for C++ solutions to competitive programming exercises\nReport issues on the bugtracker at https://github.com/MikolajKolek/toster/issues", long_about = None)]
pub struct Args {
	/// Input directory
	#[clap(short, long, value_parser, default_value = "in")]
	pub r#in: PathBuf,

	/// Input file extension
	#[clap(long, value_parser, default_value = ".in")]
	pub in_ext: String,

	/// Output directory
	#[clap(short, long, value_parser, default_value = "out")]
	pub out: PathBuf,

	/// Output file extension
	#[clap(long, value_parser, default_value = ".out")]
	pub out_ext: String,

	/// The input and output directory (sets both -i and -o at once)
	#[clap(long, value_parser)]
	pub io: Option<PathBuf>,

	/// The C++ source code or executable of a checker program that verifies if the tested program's output is correct instead of comparing it with given output files
	/// The checker must use the following protocol:
	/// - The checker receives the contents of the input file and the output of the tested program on stdin, separated by a single "\n" character
	/// - The checker outputs "C" if the output is correct, or "I <OPTIONAL_DATA>" if the output is incorrect. The optional data can include any information useful for understanding why the output is wrong and will be shown when errors are displayed
	#[clap(short, long, value_parser, verbatim_doc_comment)]
	pub checker: Option<PathBuf>,

	/// The number of seconds after which a test or generation times out if the program does not return
	#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
	#[clap(short, long, value_parser, default_value = "5")]
	pub timeout: u64,

	/// The number of seconds after which a test or generation (or checker if you're using the --checker flag) times out if the program does not return. WARNING: if you're using the sio2jail flag, this timeout will still work based on time measured directly by toster, not time measured by sio2jail
	#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
	#[clap(short, long, value_parser, default_value = "5")]
	pub timeout: u64,

	/// The number of seconds after which compilation times out if it doesn't finish
	#[clap(long, value_parser, default_value = "10")]
	pub compile_timeout: u64,

	/// The command used to compile the file. <IN> gets replaced with the path to the source code file, <OUT> is the executable output location.
	#[clap(long, value_parser, default_value = "g++ -std=c++20 -O3 -static <IN> -o <OUT>")]
	pub compile_command: String,

	/// Makes toster use sio2jail for measuring program runtime and memory use more accurately. By default limits memory use to 1 GiB. WARNING: enabling this flag can significantly slow down testing
	#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
	#[clap(short, long, action)]
	pub sio2jail: bool,

	/// Sets a memory limit (in KiB) for the executed program and enables the sio2jail flag. WARNING: enabling this flag can significantly slow down testing
	#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
	#[clap(short, long, value_parser)]
	pub memory_limit: Option<u64>,

	/// Makes toster generate output files in the output directory instead of comparing the program's output with the files in the output directory
	#[clap(short, long, action)]
	pub generate: bool,

	/// The name of the file containing the source code or the executable you want to test
	#[clap(value_parser)]
	pub filename: PathBuf
}

pub(crate) enum InputConfig {
	Directory {
		directory: PathBuf,
		ext: String,
	}
}

pub(crate) enum ExecuteMode {
	Simple,
	#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
	Sio2jail {
		memory_limit: u64,
	}
}

pub(crate) enum ActionType {
	Generate {
		output_directory: PathBuf,
		output_ext: String,
	},
	SimpleCompare {
		output_directory: PathBuf,
		output_ext: String,
	},
	Checker {
		path: PathBuf,
	}
}

pub(crate) struct ParsedConfig {
	pub(crate) source_path: PathBuf,
	pub(crate) compile_command: String,
	pub(crate) compile_timeout: Duration,
	pub(crate) execute_timeout: Duration,
	pub(crate) input: InputConfig,
	pub(crate) execute_mode: ExecuteMode,
	pub(crate) action_type: ActionType,
}

fn verify_compile_command(command: &str) -> Result<(), String> {
	let message = format!(
		"The compile command is invalid:\n{}\nRead \"toster -h\" for more info",
		match (command.contains("<IN>"), command.contains("<OUT>")) {
			(true, true) => return Ok(()),
			(false, true) => "The <IN> argument is missing\n",
			(true, false) => "The <OUT> argument is missing\n",
			(false, false) => "The <IN> and <OUT> arguments are missing\n",
		}
	);
	Err(message)
}

impl TryFrom<Args> for ParsedConfig {
	type Error = String;

	fn try_from(args: Args) -> Result<Self, String> {
		if !args.filename.is_file() {
			return Err("The provided file does not exist".to_string());
		}

		let (input_directory, output_directory) = match args.io {
			Some(io) => {
				if !io.is_dir() {
					return Err("The input/output directory does not exist".to_string());
				}
				(io.clone(), io)
			},
			None => {
				if !args.r#in.is_dir() {
					return Err("The input directory does not exist".to_string());
				}
				(args.r#in, args.out)
			}
		};

		verify_compile_command(&args.compile_command)?;

		Ok(ParsedConfig {
			source_path: args.filename,
			compile_timeout: Duration::from_secs(args.compile_timeout),
			execute_timeout: Duration::from_secs(args.timeout),
			compile_command: args.compile_command,
			input: InputConfig::Directory {
				directory: input_directory,
				ext: args.in_ext,
			},

			action_type: match (args.generate, args.checker) {
				(true, Some(_)) => {
					return Err("You can't have the --generate and --checker flags on at the same time".to_string())
				},
				(true, None) => {
					if output_directory.exists() && !output_directory.is_dir() {
						return Err("Output path is not a directory".to_string())
					}
					ActionType::Generate {
						output_directory,
						output_ext: args.out_ext,
					}
				},
				(false, None) => {
					if !output_directory.is_dir() {
						return Err("The output directory does not exist".to_string())
					}
					ActionType::SimpleCompare {
						output_directory,
						output_ext: args.out_ext,
					}
				},
				(false, Some(checker_path)) => {
					if !checker_path.is_file() {
						return Err("The provided checker file does not exist".to_string());
					}
					ActionType::Checker {
						path: checker_path,
					}
				}
			},

			execute_mode: {
				#[cfg(all(target_os = "linux", target_arch = "x86_64"))] {
					if let Some(memory_limit) = args.memory_limit {
						ExecuteMode::Sio2jail { memory_limit }
					} else if args.sio2jail {
						ExecuteMode::Sio2jail { memory_limit: 1024 * 1204 }
					} else {
						Simple
					}
				}
				#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
				Simple
			}
		})
	}
}

impl ParsedConfig {
	pub(crate) fn generate_mode(&self) -> bool {
		matches!(self.action_type, ActionType::Generate { .. })
	}
}