pub(crate) mod simple;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) mod sio2jail;

use wait_timeout::ChildExt;
use crate::prepare_input::TestInputSource;
use crate::test_errors::{ExecutionError, ExecutionMetrics};

pub(crate) trait TestExecutor: Sync + Send {
    fn test_to_string(&self, input_source: &TestInputSource) -> (ExecutionMetrics, Result<String, ExecutionError>);
}
