# Sand Post Search

Embedded Tantivy index and a semantics-preserving candidate compiler for
Sand Post's typed query language. Tantivy narrows candidates; the canonical
`sandpost-query` evaluator verifies stored facts before results are returned.

Search uses descending message sequence keysets. `authorization: None` grants
global access; a present authorization list is an OR of endpoint-bound
expressions, and an empty list denies every message.

`SearchIndex` supports in-memory and directory-backed indexes, shared readers,
writer batches of at most 256 operations, idempotent upsert/delete, commit
watermarks, and descending sequence cursors. The schema keeps exact terms for
identity, endpoint, mailbox values, headers, subject, and message ID; numeric
facts use indexed/fast fields. Text and HTML bodies are stored once in the facts
payload and use field-scoped Unicode trigrams for substring candidates, avoiding
whole-body terms. Every candidate is checked against stored facts before return.
`search_hits` returns `(MessageIdentifier, revision)` pairs so callers can
hydrate current records only when the indexed revision still matches; `search`
remains the ID-only convenience API. Adding the stored revision field changes
the schema, so older index generations must be rebuilt.

Indexing deduplicates rolling Unicode trigrams before allocating term strings,
so temporary trigram memory scales with the number of distinct triples rather
than body length times per-trigram string overhead. Trigrams increase index
size with distinct body substrings. Literals shorter
than three Unicode scalars, negation, and some ordered string predicates can
scan a broad Tantivy candidate set; they never trigger a SQL scan or change DSL
semantics. Stored facts and attachment metadata remain in each Tantivy
document, so index size grows with indexed mail content.
