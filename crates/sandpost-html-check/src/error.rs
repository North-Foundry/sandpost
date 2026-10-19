//! Errors of the compatibility check.

use thiserror::Error;

/// Why an HTML body could not be checked.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum HtmlCheckError {
    #[error("the HTML body is {size} bytes, more than the {limit} bytes that can be checked")]
    TooLarge { size: usize, limit: usize },
}
