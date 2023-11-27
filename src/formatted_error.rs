use std::fmt::{Display, Formatter};
use colored::Colorize;

pub(crate) struct FormattedError(String);

impl FormattedError {
    pub(crate) fn preformatted(formatted_string: String) -> FormattedError {
        FormattedError(formatted_string)
    }

    pub(crate) fn from_str(string: &str) -> FormattedError {
        FormattedError(string.red().to_string())
    }
}

impl Display for FormattedError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
