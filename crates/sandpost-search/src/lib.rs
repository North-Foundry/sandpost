#![doc = include_str!("../README.md")]

mod document;
mod index;
mod keys;
mod query;
mod schema;
mod search;

pub use document::IndexedDocument;
pub use index::{SearchError, SearchIndex};
pub use query::{MAXIMUM_SEARCH_BATCH_SIZE, MessageQuery};

#[cfg(test)]
mod tests;
