#![doc = include_str!("../README.md")]

mod connection;
mod error;
mod message_indexes;
mod messages;
mod migrations;
mod records;
mod schema;
mod scopes;
mod visibility;

pub use connection::Storage;
pub use error::StorageError;
pub use records::MessageSummary;

#[cfg(test)]
mod tests;
