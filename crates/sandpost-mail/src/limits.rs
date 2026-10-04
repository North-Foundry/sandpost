//! Shared bounds for parsing and mail protocol transport.
use std::time::Duration;

pub(crate) const MAXIMUM_MESSAGE_SIZE: usize = 10 * 1024 * 1024;
pub(crate) const MAXIMUM_LINE_SIZE: usize = 1000;
pub(crate) const MAXIMUM_RECIPIENT_COUNT: usize = 100;
pub(crate) const MAXIMUM_CONNECTION_COUNT: usize = 32;
pub(crate) const INPUT_OUTPUT_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
