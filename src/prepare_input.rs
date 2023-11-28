use std::fs::{File, read_dir};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator};
use rayon::vec::IntoIter;
use crate::formatted_error::FormattedError;

pub(crate) enum TestInputSource {
    File(PathBuf)
}

impl TestInputSource {
    pub(crate) fn get_stdin(&self) -> Stdio {
        match self {
            TestInputSource::File(path) => Stdio::from(File::open(path).expect("Failed to open input file"))
        }
    }

    pub(crate) fn read(&self) -> impl Read {
        match self {
            TestInputSource::File(path) => { File::open(path).expect("Failed to open input file") }
        }
    }
}

pub(crate) struct Test {
    pub(crate) test_name: String,
    pub(crate) input_source: TestInputSource,
}

pub(crate) struct TestingInputs<T: IndexedParallelIterator<Item = Test>> {
    pub(crate) test_count: usize,
    pub(crate) iterator: T,
}

pub(crate) fn prepare_file_inputs(input_dir: &Path, in_ext: &str) -> Result<TestingInputs<IntoIter<Test>>, FormattedError> {
    let tests: Vec<Test> = read_dir(&input_dir)
        .expect("Cannot open input directory!")
        .map(|input| {
            input.expect("Failed to read contents of input directory").path()
        })
        .filter(|path| {
            return match path.extension() {
                None => false,
                Some(ext) => ".".to_owned() + &ext.to_str().unwrap_or("") == in_ext
            };
        })
        .map(|file_path| {
            let test_name = file_path.file_stem().expect(&format!("The input file {} is invalid!", file_path.display())).to_str().expect(&format!("The input file {} is invalid!", file_path.display())).to_string();
            Test {
                test_name,
                input_source: TestInputSource::File(file_path)
            }
        })
        .collect();

    if tests.is_empty() {
        return Err(FormattedError::from_str("There are no files in the input directory with the provided file extension"));
    }

    let test_count = tests.len();

    Ok(TestingInputs { test_count, iterator: tests.into_par_iter() })
}