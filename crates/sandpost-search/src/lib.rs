//! Embedded Tantivy search with exact verification through the canonical DSL evaluator.

mod index;
mod keys;
mod query;

pub use index::{IndexedDocument, SearchError, SearchIndex};
pub use query::{EndpointQuery, MAXIMUM_SEARCH_BATCH_SIZE, MessageQuery};

#[cfg(test)]
mod tests;
