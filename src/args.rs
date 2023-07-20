use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "Toster", version, about = "A simple-as-toast tester for C++ solutions to competitive programming exercises\nReport issues on the bugtracker at https://github.com/MikolajKolek/toster/issues", long_about = None)]
pub struct Args {
	/// Input directory
	#[clap(short, long, value_parser, default_value = "in")]
	pub r#in: String,

	/// Input file extension
	#[clap(long, value_parser, default_value = ".in")]
	pub in_ext: String,

	/// Output directory
	#[clap(short, long, value_parser, default_value = "out")]
	pub out: String,

	/// Output file extension
	#[clap(long, value_parser, default_value = ".out")]
	pub out_ext: String,

	/// The input and output directory (sets both -i and -o at once)
	#[clap(long, value_parser)]
	pub io: Option<String>,

	/// The number of seconds after which a test or generation times out if the program does not return
	#[clap(short, long, value_parser, default_value = "5")]
	pub timeout: u64,

	/// The number of seconds after which compilation times out if it doesn't finish
	#[clap(long, value_parser, default_value = "10")]
	pub compile_timeout: u64,

	/// The command used to compile the file. <IN> gets replaced with the path to the source code file, <OUT> is the executable output location.
	#[clap(short, long, value_parser, default_value = "g++ -std=c++17 -O3 -static <IN> -o <OUT>")]
	pub compile_command: String,

	/// Makes toster use sio2jail for measuring program runtime and memory use more accurately. WARNING: enabling this flag can significantly slow down testing
	#[cfg(target_os = "linux")]
	#[cfg(target_arch = "x86_64")]
	#[clap(short, long, action)]
	pub sio2jail: bool,

	/// Sets a memory limit (in KiB) for the executed program. WARNING: enabling this flag can significantly slow down testing
	#[cfg(target_os = "linux")]
	#[cfg(target_arch = "x86_64")]
	#[clap(short, long, value_parser)]
	pub memory_limit: Option<u64>,

	/// Makes toster generate output files in the output directory instead of comparing the program's output with the files in the output directory
	#[clap(short, long, action)]
	pub generate: bool,

	/// The name of the file containing the source code or the executable you want to test
	#[clap(value_parser)]
	pub filename: String
}