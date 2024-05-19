use std::ffi::OsStr;
use std::fs::{File, read_dir};
use std::path::{Path, PathBuf};
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator};
use rayon::vec::IntoIter;
use crate::formatted_error::FormattedError;
use crate::generic_utils::ResultExt;

pub(crate) enum TestInputSource {
    File(PathBuf)
}

impl TestInputSource {
    pub(crate) fn get_file(&self) -> File {
        match self {
            TestInputSource::File(path) => { File::open(path).expect("Failed to open input file") }
        }
    }
}

pub(crate) struct Test {
    pub(crate) test_name: String,
    pub(crate) input_source: TestInputSource,
}

pub(crate) struct TestingInputs<T: IndexedParallelIterator<Item=Test>> {
    pub(crate) test_count: usize,
    pub(crate) iterator: T,
}

pub(crate) fn prepare_file_inputs(input_dir: &Path, in_ext: &str) -> Result<TestingInputs<IntoIter<Test>>, FormattedError> {
    let tests = read_dir(input_dir)
        .map_err(|error| FormattedError::from_str(&format!("Cannot open input directory:\n{error}")))?
        .map(|input| -> Result<PathBuf, FormattedError> {
            let input = input
                .map_err(|error| FormattedError::from_str(
                    &format!("Failed to read contents of input directory:\n{error}")
                ))?;
            Ok(input.path())
        })
        // This `filter` and `map` could be replaced by `filter_ok` and `map_ok`
        // in the `itertools` crate
        .filter(|path| {
            path.is_err_or(|path| {
                path.extension().is_some_and(|ext| {
                    ".".to_owned() + ext.to_str().unwrap_or("") == in_ext
                })
            })
        })
        .map(|file_path| {
            file_path.and_then(|file_path| {
                let test_name = file_path.file_stem()
                    .and_then(OsStr::to_str)
                    .ok_or(FormattedError::from_str(&format!("The input file {} is invalid", file_path.display())))?
                    .to_owned();
                Ok(Test {
                    test_name,
                    input_source: TestInputSource::File(file_path),
                })
            })
        })
        .collect::<Result<Vec<Test>, FormattedError>>()?;

    if tests.is_empty() {
        return Err(FormattedError::from_str("There are no files in the input directory with the provided file extension"));
    }

    let test_count = tests.len();

    Ok(TestingInputs { test_count, iterator: tests.into_par_iter() })
}
