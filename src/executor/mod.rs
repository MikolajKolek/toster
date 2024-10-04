pub(crate) mod simple;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) mod sio2jail;
mod common;

use std::fs::File;
use std::io::{Read, Seek};
use crate::executor::simple::SimpleExecutor;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use crate::executor::sio2jail::Sio2jailExecutor;
use crate::temp_files::create_temp_file;
use crate::test_errors::{ExecutionError, ExecutionMetrics};

pub(crate) trait TestExecutor: Sync + Send {
    /// Executes the program.
    ///
    /// Stdin is read from `input_file`, stderr is ignored.
    /// Stdout is written to `output_file`.
    /// `input_file` might not be read fully. `output_file` **is not** rewound.
    fn test_to_file(&self, input_file: &File, output_file: &File) -> (ExecutionMetrics, Result<(), ExecutionError>);
}

/// Creates a tempfile for stdout and executes the program.
///
/// Returns execution metrics and output file (if there are no errors during execution).
///
/// Stdin is read from `input_file`, stderr is ignored.
/// `input_file` might not be read fully. Output file **is** rewound before returning.
pub(crate) fn test_to_temp(executor: &impl TestExecutor, input_file: &File) -> (ExecutionMetrics, Result<impl Read, ExecutionError>) {
    let mut stdout_memfile = create_temp_file().expect("Failed to create memfile");
    let (metrics, result) = executor.test_to_file(
        input_file,
        &stdout_memfile,
    );
    stdout_memfile.rewind().expect("Failed to rewind memfile");
    (metrics, result.map(|_| stdout_memfile))
}

pub(crate) enum AnyTestExecutor {
    Simple(SimpleExecutor),
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    Sio2Jail(Sio2jailExecutor),
}

impl TestExecutor for AnyTestExecutor {
    fn test_to_file(&self, input_file: &File, output_file: &File) -> (ExecutionMetrics, Result<(), ExecutionError>) {
        match self {
            AnyTestExecutor::Simple(executor) => executor.test_to_file(input_file, output_file),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            AnyTestExecutor::Sio2Jail(executor) => executor.test_to_file(input_file, output_file),
        }
    }
}
