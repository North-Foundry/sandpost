//! Tantivy schema, durable index lifecycle, and verified keyset search.

use crate::{EndpointQuery, MessageQuery, keys, query::candidate_query};
use sandpost_core::{
    Attachment, EndpointIdentifier, MessageFacts, MessageIdentifier, MessageSequence,
};
use sandpost_query::Field as QueryField;
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tantivy::{
    Index, IndexMeta, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, TantivyError, Term,
    collector::TopDocs,
    query::{BooleanQuery, Occur, Query, TermQuery},
    schema::{
        FAST, Field, INDEXED, IndexRecordOption, STORED, STRING, Schema, Value as TantivyValue,
    },
};
use thiserror::Error;

const WRITER_HEAP_BYTES: usize = 50_000_000;
const MAXIMUM_INDEX_BATCH_SIZE: usize = 256;

/// Searchable facts and attachment metadata for one persisted message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedDocument {
    /// Public message identifier used for idempotent replacement and deletion.
    pub identifier: MessageIdentifier,
    /// Monotonically increasing persistence sequence used for pagination.
    pub sequence: MessageSequence,
    /// Persistence revision bound to the message facts for authorization hydration.
    pub revision: u64,
    /// Endpoint which owns or exposes this message.
    pub endpoint_identifier: EndpointIdentifier,
    /// Normalized message facts evaluated by the canonical query interpreter.
    pub facts: MessageFacts,
    /// Attachment facts retained with the indexed message document.
    pub attachments: Vec<Attachment>,
}

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

/// Tantivy field handles kept private so Tantivy types stay out of the API.
#[derive(Clone, Copy)]
pub(super) struct SearchFields {
    pub(super) identifier: Field,
    pub(super) endpoint: Field,
    pub(super) sequence: Field,
    pub(super) revision: Field,
    pub(super) marker: Field,
    pub(super) facts: Field,
    pub(super) attachments: Field,
    pub(super) exact_values: Field,
    pub(super) trigrams: Field,
    pub(super) received_at: Field,
    pub(super) size: Field,
    pub(super) attachment_count: Field,
}

/// Build the fixed schema used by new index directories.
fn schema() -> (Schema, SearchFields) {
    let mut builder = Schema::builder();
    let identifier = builder.add_text_field("identifier", STRING | STORED);
    let endpoint = builder.add_text_field("endpoint", STRING | STORED);
    let sequence = builder.add_u64_field("sequence", INDEXED | FAST | STORED);
    let revision = builder.add_u64_field("revision", STORED);
    let marker = builder.add_text_field("document_kind", STRING);
    let facts = builder.add_text_field("facts_json", STORED);
    let attachments = builder.add_text_field("attachments_json", STORED);
    let exact_values = builder.add_text_field("exact_values", STRING);
    let trigrams = builder.add_text_field("trigrams", STRING);
    let received_at = builder.add_i64_field("received_at", INDEXED | FAST | STORED);
    let size = builder.add_u64_field("size", INDEXED | FAST | STORED);
    let attachment_count = builder.add_u64_field("attachment_count", INDEXED | FAST | STORED);
    (
        builder.build(),
        SearchFields {
            identifier,
            endpoint,
            sequence,
            revision,
            marker,
            facts,
            attachments,
            exact_values,
            trigrams,
            received_at,
            size,
            attachment_count,
        },
    )
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
    let writer = index.writer(WRITER_HEAP_BYTES)?;
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

    /// Queue up to 256 idempotent replacements under one writer lock for the next commit.
    pub fn upsert_batch(&self, documents: &[IndexedDocument]) -> Result<(), SearchError> {
        if documents.len() > MAXIMUM_INDEX_BATCH_SIZE {
            return Err(SearchError::BatchTooLarge(documents.len()));
        }
        let tantivy_documents = documents
            .iter()
            .map(|document| self.tantivy_document(document))
            .collect::<Result<Vec<_>, _>>()?;
        let writer = self
            .writer
            .lock()
            .map_err(|_| SearchError::WriterPoisoned)?;
        let writer = writer.as_ref().ok_or(SearchError::WriterClosed)?;
        for (document, tantivy_document) in documents.iter().zip(tantivy_documents) {
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
        let result_limit = request.limit.min(crate::query::MAXIMUM_SEARCH_BATCH_SIZE);
        if result_limit == 0
            || matches!(request.authorization, Some(ref clauses) if clauses.is_empty())
        {
            return Ok(Vec::new());
        }
        let mut required: Vec<Box<dyn Query>> = vec![self.marker_query(keys::MESSAGE_MARKER)];
        if let Some(sequence) = request.before {
            let max_sequence = sequence.0.checked_sub(1);
            let Some(max_sequence) = max_sequence else {
                return Ok(Vec::new());
            };
            required.push(Box::new(tantivy::query::RangeQuery::new(
                std::ops::Bound::Unbounded,
                std::ops::Bound::Included(Term::from_field_u64(self.fields.sequence, max_sequence)),
            )));
        }
        if let Some(identifier) = request.message_identifier {
            required.push(Box::new(TermQuery::new(
                Term::from_field_text(self.fields.identifier, &identifier.to_string()),
                IndexRecordOption::Basic,
            )));
        }
        if let Some(endpoint) = request.endpoint_identifier {
            required.push(Box::new(self.endpoint_query(endpoint)));
        }
        required.push(candidate_query(&request.filter, self.fields));
        if let Some(authorization) = &request.authorization {
            let branches = authorization
                .iter()
                .map(|clause| self.authorization_query(clause))
                .collect::<Vec<_>>();
            required.push(Box::new(BooleanQuery::new(
                branches
                    .into_iter()
                    .map(|query| (Occur::Should, query))
                    .collect(),
            )));
        }
        let query = BooleanQuery::new(
            required
                .into_iter()
                .map(|query| (Occur::Must, query))
                .collect(),
        );
        let searcher = self.reader.searcher();
        let mut results = Vec::with_capacity(result_limit);
        let mut upper_sequence = request.before.map(|sequence| sequence.0);
        while results.len() < result_limit {
            let mut page_clauses = vec![(Occur::Must, Box::new(query.clone()) as Box<dyn Query>)];
            if let Some(upper_sequence) = upper_sequence {
                let Some(inclusive_upper) = upper_sequence.checked_sub(1) else {
                    break;
                };
                page_clauses.push((
                    Occur::Must,
                    Box::new(tantivy::query::RangeQuery::new(
                        std::ops::Bound::Unbounded,
                        std::ops::Bound::Included(Term::from_field_u64(
                            self.fields.sequence,
                            inclusive_upper,
                        )),
                    )),
                ));
            }
            let page_query = BooleanQuery::new(page_clauses);
            let hits = searcher.search(
                &page_query,
                &TopDocs::with_limit(crate::query::MAXIMUM_SEARCH_BATCH_SIZE)
                    .order_by_fast_field::<u64>("sequence", tantivy::Order::Desc),
            )?;
            if hits.is_empty() {
                break;
            }
            let mut last_sequence = None;
            for (_, address) in &hits {
                let document: TantivyDocument = searcher.doc(*address)?;
                let sequence = document
                    .get_first(self.fields.sequence)
                    .and_then(|value| value.as_u64())
                    .ok_or_else(|| TantivyError::InvalidArgument("missing sequence".into()))?;
                last_sequence = Some(sequence);
                if self.matches_request(&document, request)? {
                    let identifier = document
                        .get_first(self.fields.identifier)
                        .and_then(|value| value.as_str())
                        .ok_or_else(|| TantivyError::InvalidArgument("missing identifier".into()))?
                        .parse()
                        .map_err(|error| {
                            TantivyError::InvalidArgument(format!("invalid identifier: {error}"))
                        })?;
                    let revision = document
                        .get_first(self.fields.revision)
                        .and_then(|value| value.as_u64())
                        .ok_or_else(|| TantivyError::InvalidArgument("missing revision".into()))?;
                    results.push((identifier, revision));
                    if results.len() == result_limit {
                        break;
                    }
                }
            }
            if results.len() == result_limit || hits.len() < crate::query::MAXIMUM_SEARCH_BATCH_SIZE
            {
                break;
            }
            upper_sequence = last_sequence;
        }
        Ok(results)
    }

    /// Verify stored endpoint, authorization, and filter semantics exactly.
    fn matches_request(
        &self,
        document: &TantivyDocument,
        request: &MessageQuery,
    ) -> Result<bool, TantivyError> {
        let endpoint_identifier = document
            .get_first(self.fields.endpoint)
            .and_then(|value| value.as_str())
            .ok_or_else(|| TantivyError::InvalidArgument("missing endpoint".into()))?
            .parse()
            .map_err(|error| TantivyError::InvalidArgument(format!("invalid endpoint: {error}")))?;
        let facts: MessageFacts = serde_json::from_str(
            document
                .get_first(self.fields.facts)
                .and_then(|value| value.as_str())
                .ok_or_else(|| TantivyError::InvalidArgument("missing facts".into()))?,
        )
        .map_err(|error| TantivyError::InvalidArgument(format!("invalid stored facts: {error}")))?;
        let authorized = match &request.authorization {
            None => true,
            Some(clauses) => clauses.iter().any(|clause| {
                clause.endpoint_identifier == endpoint_identifier
                    && clause.expression.evaluate(&facts)
            }),
        };
        Ok(authorized
            && request
                .endpoint_identifier
                .is_none_or(|endpoint| endpoint == endpoint_identifier)
            && request.filter.evaluate(&facts))
    }

    /// Build the stored Tantivy representation and its safe candidate terms.
    fn tantivy_document(&self, document: &IndexedDocument) -> Result<TantivyDocument, SearchError> {
        let mut indexed = TantivyDocument::default();
        indexed.add_text(self.fields.identifier, document.identifier.to_string());
        indexed.add_text(
            self.fields.endpoint,
            document.endpoint_identifier.to_string(),
        );
        indexed.add_u64(self.fields.sequence, document.sequence.0);
        indexed.add_u64(self.fields.revision, document.revision);
        indexed.add_text(self.fields.marker, keys::MESSAGE_MARKER);
        indexed.add_text(self.fields.facts, serde_json::to_string(&document.facts)?);
        indexed.add_text(
            self.fields.attachments,
            serde_json::to_string(&document.attachments)?,
        );
        indexed.add_i64(self.fields.received_at, document.facts.received_at);
        indexed.add_u64(self.fields.size, document.facts.size);
        indexed.add_u64(
            self.fields.attachment_count,
            document.facts.attachment_count,
        );
        for (field, values) in indexed_string_values(&document.facts) {
            let field_key = keys::field_key(&field);
            let fold_case = keys::folds_ascii_case(&field);
            let body_field = matches!(field, QueryField::Text | QueryField::MarkupBody);
            for value in values {
                if !body_field {
                    indexed.add_text(
                        self.fields.exact_values,
                        keys::exact_term(&field_key, value, fold_case),
                    );
                }
                indexed.add_text(self.fields.exact_values, keys::presence_term(&field_key));
                for trigram in keys::distinct_trigrams(value, fold_case) {
                    let trigram: String = trigram.iter().copied().collect();
                    indexed.add_text(
                        self.fields.trigrams,
                        keys::trigram_term(&field_key, &trigram),
                    );
                }
            }
        }
        let content_key = keys::field_key(&QueryField::Content);
        indexed.add_text(self.fields.exact_values, keys::presence_term(&content_key));
        for value in indexed_content_values(&document.facts) {
            for trigram in keys::distinct_trigrams(value, true) {
                let trigram: String = trigram.iter().copied().collect();
                indexed.add_text(
                    self.fields.trigrams,
                    keys::trigram_term(&content_key, &trigram),
                );
            }
        }
        indexed.add_text(
            self.fields.exact_values,
            keys::exact_term(
                &keys::field_key(&QueryField::HasAttachments),
                &(document.facts.attachment_count > 0).to_string(),
                keys::folds_ascii_case(&QueryField::HasAttachments),
            ),
        );
        Ok(indexed)
    }

    /// Build a branch query that safely narrows endpoint authorization candidates.
    fn authorization_query(&self, clause: &EndpointQuery) -> Box<dyn Query> {
        Box::new(BooleanQuery::new(vec![
            (Occur::Must, self.endpoint_query(clause.endpoint_identifier)),
            (
                Occur::Must,
                candidate_query(&clause.expression, self.fields),
            ),
        ]))
    }

    /// Match documents assigned to one endpoint identifier.
    fn endpoint_query(&self, endpoint_identifier: EndpointIdentifier) -> Box<dyn Query> {
        Box::new(TermQuery::new(
            Term::from_field_text(self.fields.endpoint, &endpoint_identifier.to_string()),
            IndexRecordOption::Basic,
        ))
    }

    /// Match the requested document kind through the private marker field.
    fn marker_query(&self, marker: &str) -> Box<dyn Query> {
        Box::new(TermQuery::new(
            Term::from_field_text(self.fields.marker, marker),
            IndexRecordOption::Basic,
        ))
    }
}

/// Return every queryable stored string with its DSL field, without copying message strings.
fn indexed_string_values(facts: &MessageFacts) -> Vec<(QueryField, Vec<&str>)> {
    let mut values = vec![
        (
            QueryField::EnvelopeFromAddress,
            facts
                .envelope_from
                .iter()
                .map(|mailbox| mailbox.address.as_str())
                .collect(),
        ),
        (
            QueryField::EnvelopeFromDomain,
            facts
                .envelope_from
                .iter()
                .map(|mailbox| mailbox.domain.as_str())
                .collect(),
        ),
        (
            QueryField::EnvelopeToAddress,
            facts
                .envelope_to
                .iter()
                .map(|mailbox| mailbox.address.as_str())
                .collect(),
        ),
        (
            QueryField::EnvelopeToDomain,
            facts
                .envelope_to
                .iter()
                .map(|mailbox| mailbox.domain.as_str())
                .collect(),
        ),
        (
            QueryField::FromAddress,
            facts
                .from
                .iter()
                .map(|mailbox| mailbox.address.as_str())
                .collect(),
        ),
        (
            QueryField::FromDomain,
            facts
                .from
                .iter()
                .map(|mailbox| mailbox.domain.as_str())
                .collect(),
        ),
        (
            QueryField::ToAddress,
            facts
                .to
                .iter()
                .map(|mailbox| mailbox.address.as_str())
                .collect(),
        ),
        (
            QueryField::ToDomain,
            facts
                .to
                .iter()
                .map(|mailbox| mailbox.domain.as_str())
                .collect(),
        ),
        (
            QueryField::CarbonCopyAddress,
            facts
                .carbon_copy
                .iter()
                .map(|mailbox| mailbox.address.as_str())
                .collect(),
        ),
        (
            QueryField::CarbonCopyDomain,
            facts
                .carbon_copy
                .iter()
                .map(|mailbox| mailbox.domain.as_str())
                .collect(),
        ),
        (QueryField::Subject, vec![facts.subject.as_str()]),
        (QueryField::Text, vec![facts.text.as_str()]),
        (QueryField::MarkupBody, vec![facts.markup_body.as_str()]),
    ];
    if let Some(identifier) = &facts.message_identifier {
        values.push((QueryField::MessageIdentifier, vec![identifier]));
    }
    for (name, strings) in &facts.headers {
        values.push((
            QueryField::Header(name.clone()),
            strings.iter().map(String::as_str).collect(),
        ));
    }
    values
}

/// Return every literal-search value, including header names, without copying message strings.
fn indexed_content_values(facts: &MessageFacts) -> Vec<&str> {
    let mut values = vec![
        facts.subject.as_str(),
        facts.text.as_str(),
        facts.markup_body.as_str(),
    ];
    values.extend(
        facts
            .envelope_from
            .iter()
            .map(|mailbox| mailbox.address.as_str()),
    );
    values.extend(
        facts
            .envelope_to
            .iter()
            .map(|mailbox| mailbox.address.as_str()),
    );
    values.extend(facts.from.iter().map(|mailbox| mailbox.address.as_str()));
    values.extend(facts.to.iter().map(|mailbox| mailbox.address.as_str()));
    values.extend(
        facts
            .carbon_copy
            .iter()
            .map(|mailbox| mailbox.address.as_str()),
    );
    values.extend(facts.message_identifier.iter().map(String::as_str));
    for (name, header_values) in &facts.headers {
        values.push(name);
        values.extend(header_values.iter().map(String::as_str));
    }
    values
}
