//! Message storage conformance checks grouped by retrieval, filtering, and indexing.
mod filters;
mod index;
mod records;

pub use filters::message_filter_equivalence;
pub use index::index_messages_revisions;
pub use records::{
    message_deletion, message_ingestion_and_retrieval, message_ordering_and_pagination,
};
