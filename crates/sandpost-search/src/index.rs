//! Public index facade and durable reader/writer lifecycle.

use crate::{
    IndexedDocument, MessageQuery,
    document::tantivy_document,
    schema::{SearchFields, schema},
};
use sandpost_core::MessageIdentifier;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tantivy::{Index, IndexMeta, IndexReader, IndexWriter, ReloadPolicy, TantivyError, Term};
use thiserror::Error;

const WRITER_HEAP_BYTES: usize = 15_000_000;
const MAXIMUM_INDEX_BATCH_SIZE: usize = 256;

/// Failures returned by the embedded search index.
#[derive(Debug, Error)]
pub enum SearchError {
    /// The embedded index failed to open, update, commit, or search.
    #[error("search index error: {0}")]
    Tantivy(String),
    /// Stored message facts could not be serialized or decoded.
    #[error("search document serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    /// The index directory could not be created or accessed.
    #[error("search index filesystem error: {0}")]
    Io(#[from] std::io::Error),
    /// The index writer mutex was poisoned after a panic.
    #[error("search index writer lock was poisoned")]
    WriterPoisoned,
    /// The runtime has released this generation's exclusive writer ownership.
    #[error("search index writer is closed")]
    WriterClosed,
    /// A single index operation exceeded the supported bounded batch size.
    #[error("search batch has {0} items; maximum is 256")]
    BatchTooLarge(usize),
    /// An existing index uses a schema incompatible with this crate version.
    #[error("existing Tantivy index has an incompatible schema")]
    IncompatibleSchema,
    /// Tantivy identified one or more corrupted files in the committed index.
    #[error("corrupted search index files: {0}")]
    CorruptIndex(String),
    /// The committed watermark payload was not a valid unsigned sequence.
    #[error("invalid committed search watermark: {0}")]
    InvalidWatermark(String),
}

impl From<TantivyError> for SearchError {
    /// Convert Tantivy failures to text without exposing Tantivy types in the public API.
    fn from(error: TantivyError) -> Self {
        Self::Tantivy(error.to_string())
    }
}

/// Thread-safe embedded search index with exact DSL verification.
#[derive(Clone)]
pub struct SearchIndex {
    index: Index,
    reader: IndexReader,
    writer: Arc<Mutex<Option<IndexWriter>>>,
    fields: SearchFields,
}

/// Open or create a durable Tantivy index in `path`.
pub fn open_index(path: impl AsRef<Path>) -> Result<SearchIndex, SearchError> {
    let path = path.as_ref();
    std::fs::create_dir_all(path)?;
    let (schema, fields) = schema();
    let index = if path.join("meta.json").exists() {
        Index::open_in_dir(path)?
    } else {
        Index::create_in_dir(path, schema.clone())?
    };
    if index.schema() != schema {
        return Err(SearchError::IncompatibleSchema);
    }
    let corrupted_files = index.validate_checksum()?;
    if !corrupted_files.is_empty() {
        return Err(SearchError::CorruptIndex(
            corrupted_files
                .into_iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    finish_open(index, fields)
}

/// Create an in-memory Tantivy index.
pub fn memory_index() -> Result<SearchIndex, SearchError> {
    let (schema, fields) = schema();
    finish_open(Index::create_in_ram(schema), fields)
}

/// Attach a manual-reload reader and one writer to an opened index.
fn finish_open(index: Index, fields: SearchFields) -> Result<SearchIndex, SearchError> {
    let reader = index
        .reader_builder()
        .reload_policy(ReloadPolicy::Manual)
        .try_into()?;
    // Keep small batches on one worker to limit the number of segments per commit.
    let writer = index.writer_with_num_threads(1, WRITER_HEAP_BYTES)?;
    Ok(SearchIndex {
        index,
        reader,
        writer: Arc::new(Mutex::new(Some(writer))),
        fields,
    })
}

impl SearchIndex {
    /// Create a fresh in-memory index.
    pub fn memory() -> Result<Self, SearchError> {
        memory_index()
    }

    /// Open or create an on-disk index directory.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SearchError> {
        open_index(path)
    }

    /// Create an on-disk index, returning an error when the path already contains an index.
    pub fn create(path: impl AsRef<Path>) -> Result<Self, SearchError> {
        std::fs::create_dir_all(path.as_ref())?;
        let (schema, fields) = schema();
        finish_open(Index::create_in_dir(path, schema)?, fields)
    }

    /// Replace any indexed message with the same public identifier.
    pub fn upsert(&self, document: &IndexedDocument) -> Result<(), SearchError> {
        self.upsert_batch(std::slice::from_ref(document))
    }

    /// Queue up to 256 replacements under one writer lock, preparing one document at a time.
    /// A failed batch may leave earlier items queued; these mutations become visible on commit.
    pub fn upsert_batch(&self, documents: &[IndexedDocument]) -> Result<(), SearchError> {
        if documents.len() > MAXIMUM_INDEX_BATCH_SIZE {
            return Err(SearchError::BatchTooLarge(documents.len()));
        }
        let writer = self
            .writer
            .lock()
            .map_err(|_| SearchError::WriterPoisoned)?;
        let writer = writer.as_ref().ok_or(SearchError::WriterClosed)?;
        for document in documents {
            let tantivy_document = tantivy_document(document, self.fields)?;
            writer.delete_term(Term::from_field_text(
                self.fields.identifier,
                &document.identifier.to_string(),
            ));
            writer.add_document(tantivy_document)?;
        }
        Ok(())
    }

    /// Remove a message by public identifier; deleting an absent identifier succeeds.
    pub fn delete(&self, identifier: MessageIdentifier) -> Result<(), SearchError> {
        self.delete_batch(&[identifier])
    }

    /// Queue up to 256 identifier deletions under one writer lock; absent IDs are harmless.
    pub fn delete_batch(&self, identifiers: &[MessageIdentifier]) -> Result<(), SearchError> {
        if identifiers.len() > MAXIMUM_INDEX_BATCH_SIZE {
            return Err(SearchError::BatchTooLarge(identifiers.len()));
        }
        let writer = self
            .writer
            .lock()
            .map_err(|_| SearchError::WriterPoisoned)?;
        let writer = writer.as_ref().ok_or(SearchError::WriterClosed)?;
        for identifier in identifiers {
            writer.delete_term(Term::from_field_text(
                self.fields.identifier,
                &identifier.to_string(),
            ));
        }
        Ok(())
    }

    /// Commit all pending changes with the storage sequence watermark as commit payload.
    pub fn commit(&self, watermark: u64) -> Result<(), SearchError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| SearchError::WriterPoisoned)?;
        let writer = writer.as_mut().ok_or(SearchError::WriterClosed)?;
        let mut prepared = writer.prepare_commit()?;
        prepared.set_payload(&watermark.to_string());
        prepared.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    /// Release the exclusive Tantivy writer while retaining the committed reader snapshot.
    /// Further mutations fail explicitly, including through cloned handles.
    pub fn close_writer(&self) -> Result<(), SearchError> {
        self.writer
            .lock()
            .map_err(|_| SearchError::WriterPoisoned)?
            .take();
        Ok(())
    }

    /// Return the sequence watermark stored in the last successful commit.
    pub fn committed_sequence(&self) -> Result<u64, SearchError> {
        let metadata: IndexMeta = self.index.load_metas()?;
        match metadata.payload {
            None => Ok(0),
            Some(payload) => payload
                .parse()
                .map_err(|_| SearchError::InvalidWatermark(payload)),
        }
    }

    /// Search newest-first and return only message identifiers.
    pub fn search(&self, request: &MessageQuery) -> Result<Vec<MessageIdentifier>, SearchError> {
        Ok(self
            .search_hits(request)?
            .into_iter()
            .map(|(identifier, _)| identifier)
            .collect())
    }

    /// Search newest-first and return each ID with the indexed facts revision.
    pub fn search_hits(
        &self,
        request: &MessageQuery,
    ) -> Result<Vec<(MessageIdentifier, u64)>, SearchError> {
        crate::search::search_hits(&self.reader, self.fields, request)
    }
}
