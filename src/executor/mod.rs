pub(crate) mod simple;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) mod sio2jail;

use std::io::{Read, Seek};
use std::process::Stdio;
use memfile::MemFile;
use crate::executor::simple::SimpleExecutor;
use crate::executor::sio2jail::Sio2jailExecutor;
use crate::test_errors::{ExecutionError, ExecutionMetrics};

pub(crate) trait TestExecutor: Sync + Send {
    fn test_to_stdio(&self, input_stdio: Stdio, output_stdio: Stdio) -> (ExecutionMetrics, Result<(), ExecutionError>);
}

pub(crate) fn test_to_temp(executor: &impl TestExecutor, input_stdio: Stdio) -> (ExecutionMetrics, Result<impl Read, ExecutionError>) {
    let mut stdout_memfile = MemFile::create_default("Program output memfile").expect("Failed to create memfile");
    let (metrics, result) = executor.test_to_stdio(
        input_stdio,
        Stdio::from(stdout_memfile.try_clone().unwrap()),
    );
    stdout_memfile.rewind().expect("Failed to seek memfile");
    (metrics, result.map(|_| stdout_memfile))
}

pub(crate) enum AnyTestExecutor {
    Simple(SimpleExecutor),
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    Sio2Jail(Sio2jailExecutor),
}

impl TestExecutor for AnyTestExecutor {
    fn test_to_stdio(&self, input_stdio: Stdio, output_stdio: Stdio) -> (ExecutionMetrics, Result<(), ExecutionError>) {
        match self {
            AnyTestExecutor::Simple(executor) => executor.test_to_stdio(input_stdio, output_stdio),
            AnyTestExecutor::Sio2Jail(executor) => executor.test_to_stdio(input_stdio, output_stdio),
        }
    }
}
