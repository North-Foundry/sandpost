# Sand Post Search

Embedded Tantivy search for SandPost's typed query language. The public API exposes
only domain types. `index.rs` owns the reader/writer lifecycle, `schema.rs` the
private schema, `document.rs` fact encoding, `query.rs` candidate planning, and
`search.rs` snapshot execution and keyset pagination.

## Usage and lifecycle

```rust
use sandpost_core::{MessageFacts, MessageIdentifier, MessageSequence};
use sandpost_query::Expression;
use sandpost_search::{IndexedDocument, MessageQuery, SearchIndex};

let index = SearchIndex::memory()?;
index.upsert(&IndexedDocument {
    identifier: MessageIdentifier::new(),
    sequence: MessageSequence(1),
    revision: 1,
    facts: MessageFacts::default(),
})?;
index.commit(1)?;
let hits = index.search_hits(&MessageQuery {
    authorization: None,
    filter: Expression::True,
    message_identifier: None,
    before: None,
    limit: 50,
})?;
# Ok::<(), sandpost_search::SearchError>(())
```

`memory` is ephemeral; `open(path)` opens or creates a durable index; `create(path)`
requires a new index. Upserts and deletes queue mutations until `commit(watermark)`
publishes them to shared readers. Each writer call accepts at most 256 items,
preparing one document at a time rather than retaining the encoded batch. A failed
batch can leave earlier items queued. `close_writer` releases writer ownership for
all clones while retaining committed readers. It does not commit pending changes.
Each indexed message must have a unique persistence sequence for keyset pagination.

`search_hits` returns `(MessageIdentifier, revision)` pairs, newest first. `search`
returns IDs only. `before` is an exclusive sequence cursor; `limit` is capped at
`MAXIMUM_SEARCH_BATCH_SIZE` (256). `authorization: None` allows all mail;
`Some(Expression::False)` denies everything. Application callers hydrate results
only when the current storage revision still matches the indexed revision.

## Exactness and performance

The schema stores only canonical message facts. Identity and revision are fast
columns; sequence and numeric facts use indexed/fast columns. It does not duplicate
attachment metadata or numeric values in stored documents, or keep a document-kind
marker. `has_attachments` uses the attachment-count column directly, without a redundant boolean term. Attachment counts remain queryable through facts and their numeric column.
Attachment metadata and bytes remain in authoritative storage.

Boolean constants, supported numeric comparisons, attachment-presence comparisons,
and AND/OR compositions consisting entirely of these predicates have exact index
plans. They read result columns without fetching or deserializing stored bodies.
All other plans are candidate supersets and evaluate the canonical filter and
authorization against stored facts before returning results. Oversized terms fall
back to broader candidates instead of losing matches at Tantivy's token limit.

Text and HTML bodies use field-scoped Unicode trigrams instead of whole-body
terms. Repeated presence terms and aggregate-content trigrams are deduplicated.
Literals shorter than three Unicode scalars, negations, and ordered string
predicates may still scan broadly; 256 limits result/page sizes, not total work.
Full facts are decoded for candidates requiring verification. Corpus size, body
sizes, and distinct substrings determine index size and workload costs.

Schema changes make older generations incompatible. The application automatically
rebuilds the derived index from authoritative storage; reopening an incompatible
index directly returns `SearchError::IncompatibleSchema`.

Run `cargo test -p sandpost-search` for semantic, persistence, boundary, and
concurrency coverage. `cargo run -p sandpost-search --example search_benchmark`
prints warm-request medians for a fixed 400-message corpus; these local timings
are a comparison tool, not a throughput guarantee.
