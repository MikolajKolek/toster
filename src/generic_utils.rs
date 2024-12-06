use std::thread;
use std::time::Duration;

#[deprecated(note = "This is not ideal, there must be a better way to implement it")]
pub(crate) fn halt() -> ! {
    thread::sleep(Duration::from_secs(u64::MAX));
    unreachable!()
}