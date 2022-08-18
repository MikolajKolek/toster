use std::{fs};
use std::cmp::max;
use std::env::current_dir;
use std::fmt::{Write as FmtWrite};
use std::fs::{File, read_dir};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ansi_term::Color::{Green, Red};
use atomic_counter::{AtomicCounter, RelaxedCounter};
use clap::Parser;
use indicatif::{ParallelProgressIterator, ProgressState, ProgressStyle};
use lazy_static::lazy_static;
use rayon::iter::{IntoParallelRefIterator};
use rayon::prelude::*;
use term_table::{Table, TableStyle};
use term_table::row::Row;
use term_table::table_cell::{Alignment, TableCell};
use wait_timeout::ChildExt;

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

    /// The number of seconds after which a test or generation times out if the program does not return
    #[clap(short, long, value_parser, default_value = "5")]
    timeout: u64,

    /// Makes the tester generate output files in the output directory instead of comparing the program's output with the files in the output directory
    #[clap(short, long, action)]
    generate: bool,

    /// The name of the file containing the source code
    #[clap(value_parser, default_value = "dom.cpp")]
    filename: String
}

lazy_static! {
    static ref CORRECT: RelaxedCounter = RelaxedCounter::new(0);
    static ref INCORRECT: RelaxedCounter = RelaxedCounter::new(0);
}

fn main() {
    let args = Args::parse();
    let workspace_dir = current_dir().unwrap().to_str().unwrap().to_string();
    let input_dir: String = format!("{}/{}", &workspace_dir, args.r#in);
    let output_dir: String = format!("{}/{}", &workspace_dir, args.out);
    let executable = format!("{}.o", Path::new(&args.filename).file_stem().unwrap().to_str().unwrap());

    Command::new("g++")
        .args(["-std=c++17", "-O3", "-static", &args.filename, "-o", &executable])
        .output().unwrap();

    let mut style: ProgressStyle = ProgressStyle::with_template("[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({eta})\n{correct} {incorrect}")
        .unwrap()
        .with_key("eta", |state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
        .progress_chars("#>-");

    if !args.generate {
        style = style.with_key("correct", |_state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{}", Green.paint(format!("{} correct", &CORRECT.get()))).unwrap())
            .with_key("incorrect", |_state: &ProgressState, w: &mut dyn FmtWrite| write!(w, "{}", Red.paint(format!("{} incorrect", &INCORRECT.get()))).unwrap());
    }

    let slowest_test: Arc<Mutex<(f64, String)>> = Arc::new(Mutex::new((-1 as f64, "PLACEHOLDER".parse().unwrap())));
    let errors = Arc::new(Mutex::new(vec![]));
    let before_testing = Instant::now();
    read_dir(&input_dir).unwrap().collect::<Vec<_>>().par_iter().progress_with_style(style).for_each(|file| {
        let input_file = File::open(file.as_ref().unwrap().path()).unwrap();

        let start = Instant::now();
        let mut child = Command::new(format!("./{}", &executable))
            .stdout(Stdio::piped())
            .stdin(input_file)
            .spawn().unwrap();
        let output_str = match child.wait_timeout(Duration::from_secs(args.timeout)).unwrap() {
            Some(_) => {
                let mut res = String::new();
                child.stdout.unwrap().read_to_string(&mut res).unwrap();
                res
            }
            None => {
                child.kill().unwrap();
                "The program timed out".to_string()
            }
        };
        let test_time = start.elapsed().as_secs_f64();

        let test_name = file.as_ref().unwrap().path().file_stem().unwrap().to_str().unwrap().to_string();
        let clone = Arc::clone(&slowest_test);
        let mut slowest_test_mutex = clone.lock().unwrap();
        if test_time > slowest_test_mutex.0 {
            *slowest_test_mutex = (test_time, test_name.clone());
        }

        let output_file = format!("{}/{}.out", &output_dir, test_name);
        if !args.generate {
            let output_file_contents = fs::read_to_string(Path::new(&output_file)).unwrap();

            if output_str.split_whitespace().collect::<Vec<&str>>() != output_file_contents.split_whitespace().collect::<Vec<&str>>() {
                INCORRECT.inc();
                let clone = Arc::clone(&errors);
                clone.lock().unwrap().push((test_name, output_str, output_file_contents));
            }
            else {
                CORRECT.inc();
            }
        }
        else {
            fs::write(Path::new(&output_file), output_str).unwrap();
        }
    });



    let slowest_test_clone = Arc::clone(&slowest_test);
    let slowest_test_mutex = slowest_test_clone.lock().unwrap();
    if !args.generate {
        println!("Testing finished in {:.2}s with {} and {}: (Slowest test: {} at {:.3}s)",
                 before_testing.elapsed().as_secs_f64(),
                 Green.paint(format!("{} correct answers", CORRECT.get())),
                 Red.paint(format!("{} incorrect answers", INCORRECT.get())),
                 slowest_test_mutex.1,
                 slowest_test_mutex.0
        );

        let errors_clone = Arc::clone(&errors);
        let errors_mutex = errors_clone.lock().unwrap();
        if !errors_mutex.is_empty() {
            println!("Errors were found in the following tests:");

            for (test_name, program_out, file_out) in errors_mutex.iter() {
                println!("Test {}:", test_name);

                let split_file = file_out.split("\n").collect::<Vec<_>>();
                let split_out = program_out.split("\n").collect::<Vec<_>>();
                if max(split_file.len(), split_out.len()) <= 100 {
                    let mut table = Table::new();
                    table.max_column_width = 40;
                    table.style = TableStyle::extended();

                    table.add_row(Row::new(vec![
                        TableCell::new(Green.paint("Output file")),
                        TableCell::new_with_alignment(Red.paint("Your program's output"), 1, Alignment::Right)
                    ]));

                    for i in 0..max(split_file.len(), split_out.len()) {
                        let file_segment = if split_file.len() > i { split_file[i] } else { "" };
                        let out_segment = if split_out.len() > i { split_out[i] } else { "" };

                        if file_segment != out_segment {
                            table.add_row(Row::new(vec![
                                TableCell::new(Green.paint(file_segment)),
                                TableCell::new_with_alignment(Red.paint(out_segment), 1, Alignment::Right)
                            ]));
                        }
                        else {
                            table.add_row(Row::new(vec![
                                TableCell::new(file_segment),
                                TableCell::new_with_alignment(out_segment, 1, Alignment::Right)
                            ]));
                        }
                    }

                    println!("{}", table.render());
                }
                else {
                    println!("{}", Red.paint(""))
                }
            }
        }
    }
    else {
        println!("Program finished in {:.2}s (Slowest test: {} at {:.3}s)",
                 before_testing.elapsed().as_secs_f64(),
                 slowest_test_mutex.1,
                 slowest_test_mutex.0
        )
    }
}