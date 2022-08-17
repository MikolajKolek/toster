use std::{fs};
use std::env::current_dir;
use std::fmt::{Write as FmtWrite};
use std::fs::{File, read_dir};
use std::path::Path;
use std::process::Command;
use std::time::{Instant};
use ansi_term::Color::{Green, Red};
use atomic_counter::{AtomicCounter, RelaxedCounter};
use clap::Parser;
use indicatif::{ParallelProgressIterator, ProgressState, ProgressStyle};
use lazy_static::lazy_static;
use rayon::iter::IntoParallelRefIterator;
use rayon::prelude::*;

/// A simple tester for competitive programming exercises
#[derive(Parser, Debug)]
#[clap(name = "Tester", version, about, long_about = None)]
struct Args {
    /// Input directory
    #[clap(short, long, value_parser, default_value = "in")]
    r#in: String,

    /// Output directory
    #[clap(short, long, value_parser, default_value = "out")]
    out: String,

    /// The name of the file containing the source code
    #[clap(value_parser)]
    filename: String
}

lazy_static! {
    static ref CORRECT: RelaxedCounter = RelaxedCounter::new(0);
    static ref INCORRECT: RelaxedCounter = RelaxedCounter::new(0);
}

fn main() {
    let args = Args::parse();
    let input_dir: String = format!("{}/{}", current_dir().unwrap().to_str().unwrap(), args.r#in);
    let output_dir: String = format!("{}/{}", current_dir().unwrap().to_str().unwrap(), args.out);
    let executable = format!("{}.o", Path::new(&args.filename).file_stem().unwrap().to_str().unwrap());

    Command::new("g++")
        .args(["-std=c++17", "-O3", "-static", &args.filename, "-o", &executable])
        .output().unwrap();

    let style: ProgressStyle = ProgressStyle::with_template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({eta})\n{correct} {incorrect}")
        .unwrap()
        .with_key("eta", |state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
        .with_key("correct", |_state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{}", Green.paint(format!("{} correct", &CORRECT.get()))).unwrap())
        .with_key("incorrect", |_state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{}", Red.paint(format!("{} incorrect", &INCORRECT.get()))).unwrap())
        .progress_chars("#>-");

    let before_testing = Instant::now();
    read_dir(&input_dir).unwrap().collect::<Vec<_>>().par_iter().progress_with_style(style).for_each(
    |file| {
        let input_file = File::open(file.as_ref().unwrap().path()).unwrap();
        let output = Command::new(format!("./{}", &executable))
            .stdin(input_file)
            .output().unwrap();
        let output_str = String::from_utf8(output.stdout).unwrap();

        let output_file = format!("{}/{}.out", &output_dir, file.as_ref().unwrap().path().file_stem().unwrap().to_str().unwrap());
        let output_file_contents = fs::read_to_string(Path::new(&output_file)).unwrap();

        if output_str.split_whitespace().collect::<Vec<&str>>() != output_file_contents.split_whitespace().collect::<Vec<&str>>() {
            INCORRECT.inc();
        }
        else {
            CORRECT.inc();
        }
    });

    println!("Testing finished in {:.2}s with {} and {}",
        before_testing.elapsed().as_secs_f64(),
        Green.paint(format!("{} correct answers", CORRECT.get())),
        Red.paint(format!("{} incorrect answers", INCORRECT.get()))
    )
}