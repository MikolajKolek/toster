use clap::Parser;

/// A simple-as-toast tester for C++ solutions to competitive programming exercises
#[derive(Parser, Debug)]
#[clap(name = "Toster", version, about, long_about = None)]
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

    /// Makes the tester generate output files in the output directory instead of comparing the program's output with the files in the output directory
    #[clap(short, long, action)]
    pub generate: bool,

    /// The name of the file containing the source code
    #[clap(value_parser)]
    pub filename: String
}