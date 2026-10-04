#![doc = include_str!("../README.md")]

mod limits;
mod message_parsing;
mod protocol_stream;
mod server;
mod session;

pub use message_parsing::parse_message;
pub use server::serve;

use thiserror::Error;

/// Parsing failures and transport errors reported by the public mail interface.
#[derive(Debug, Error)]
pub enum MailError {
    #[error("invalid message: {0}")]
    InvalidMessage(String),
    #[error("SMTP I/O: {0}")]
    InputOutput(#[from] std::io::Error),
}

#[cfg(test)]
mod tests;
