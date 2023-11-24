pub(crate) trait OptionExt<T> {
    fn is_none_or<F: FnOnce(T) -> bool>(&self, fun: F) -> bool;
}

impl<T> OptionExt<T> for Option<T> {
    fn is_none_or<F: FnOnce(&T) -> bool>(&self, fun: F) -> bool {
        match self {
            None => true,
            Some(val) => fun(val)
        }
    }
}