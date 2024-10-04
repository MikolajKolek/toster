use std::thread;
use std::time::Duration;

pub(crate) trait OptionExt<T> {
    fn is_none_or<F: FnOnce(&T) -> bool>(&self, fun: F) -> bool;
}

impl<T> OptionExt<T> for Option<T> {
    fn is_none_or<F: FnOnce(&T) -> bool>(&self, fun: F) -> bool {
        match self {
            None => true,
            Some(val) => fun(val)
        }
    }
}

pub(crate) trait ResultExt<T> {
    fn is_err_or<F: FnOnce(&T) -> bool>(&self, fun: F) -> bool;
}

impl<T, E> ResultExt<T> for Result<T, E> {
    fn is_err_or<F: FnOnce(&T) -> bool>(&self, fun: F) -> bool {
        match self {
            Err(_) => true,
            Ok(val) => fun(val)
        }
    }
}

#[deprecated(note = "This is not ideal, there must be a better way to implement it")]
pub(crate) fn halt() -> ! {
    thread::sleep(Duration::from_secs(u64::MAX));
    unreachable!()
}
