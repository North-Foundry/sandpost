# Storage

This guide is the authoritative agent reference for persisted data and database
behavior. The current backend is SQLite, provided by `sandpost-storage`. The
storage boundary owns schema initialization and validation, transactional writes,
prepared reads, decoding, and persistence of scopes, messages, and materialized
scope visibility. Domain types remain in core; callers exchange domain records
rather than SQL rows. SQLite and a writable data directory are the baseline
infrastructure.

## SQLite: implemented behavior

### Startup and schema baseline

Schema baseline 1 consists of ten ordered SQL migration files, all recorded in
`migrations` with batch 1. `PRAGMA user_version` is set to 1. Initialization
runs in an immediate write transaction:

- A fresh, empty version 0 database receives the baseline.
- A version 1 database is reopened only when its migration history exactly
  matches the ten expected entries and its tables, indexes, constraints, views,
  and triggers match the expected schema.
- Negative or newer schema versions, a populated version 0 database, missing
  migration history, mismatched history, or a changed/incomplete schema are
  rejected. There is no legacy-schema compatibility or automatic repair path.

Every connection enables foreign keys, requests WAL journal mode, and uses a
five-second busy timeout. The timeout bounds waiting on SQLite locks; it does
not add parallel writers.

### Relational data model

The ten tables are:

| Table | Stored facts and relationships |
| --- | --- |
| `migrations` | Applied migration filename and positive batch number. |
| `scopes` | Scope identity, optional parent, descriptive fields, filter, ordering position, and nonnegative policy version. The parent foreign key has no deletion cascade. |
| `users` | User identity, name, and optional personal filter. |
| `memberships` | User-to-scope membership and role (`owner`, `admin`, `member`, or `viewer`); deleting either parent cascades. |
| `inboxes` | User-owned inbox identity, name, and filter; deleting the user cascades. |
| `mail` | One row per message: numeric sequence, unique public identifier, subject, text and markup bodies, optional message identifier, original raw message bytes, received time, and nonnegative size. |
| `mail_recipients` | Ordered mailbox facts with role, address, and domain. Roles preserve envelope sender, envelope recipients, MIME From, To, and Cc separately. |
| `mail_headers` | Ordered values by message and header name, preserving repeated header values. |
| `mail_scope` | Materialized message-to-scope visibility with the policy version used to compute each match. |
| `mail_attachments` | Ordered attachment metadata and content hash, including optional filename, content type, and nonnegative size. |

Child mail facts reference `mail.sequence` and cascade on message deletion.
Recipient role and ordinal are part of the primary key, so duplicates at
separate positions and original order are retained. The envelope sender has
ordinal zero and at most one row for a message. Header values are ordered within
each name. Attachment count is derived from attachment rows, not stored as a
separate fact. Indexes support scope adjacency, membership/inbox lookup, mail
sequence/time ordering, recipient address/domain lookup by role, header lookup,
and scope-to-message access. These are candidate and lookup indexes; they do
not constitute a complete index for every query predicate.

The recipient role values are `envelope_from`, `envelope_to`, `from`, `to`, and
`carbon_copy`. Users, memberships, and inboxes have schema tables, but their
domain records do not yet have persistence methods in the storage API.

There are no JSON-encoded mail facts in SQLite. The original MIME message is
stored as a BLOB. Extracted attachment data currently consists of metadata and a
content hash; attachment bytes can be recovered from the raw MIME message, and
are not stored as separate blobs.

### Transactions and visibility consistency

Scope writes reject decreasing policy versions. Changing a stored filter or
parent requires a higher version; changing descriptive fields can keep the same
version. Policy evaluation and authorization remain the caller's responsibility: a
visibility query does not verify access rights to the supplied scopes.

Message insertion stores the mail row, recipient/header/attachment rows, and
provided scope matches in one transaction. Before writing each match, storage
checks that the scope still has the policy version used by the matcher. A stale
version or any insert/validation failure aborts the transaction, so neither a
partial message nor partial visibility links are committed. Callers acknowledge
ingestion and publish notifications only after persistence succeeds.

Replacing a scope's materialized match set checks the current policy version,
then deletes and inserts that scope's links in a single transaction. Visibility
reads include only links whose stored policy version still equals the current
scope version. Queries across multiple scopes deduplicate messages, since a
message may be visible through more than one scope. Foreign keys enforce the
referential links to scopes and messages.

### Reads, bounds, and integer conversion

Summary reads select scalar message fields and attachment counts without
loading bodies or raw MIME bytes. Mailbox relations are loaded for the selected
page. Full message reads include raw bytes; metadata reads do not. Message lists
are newest-first and use a sequence cursor, with a maximum page size of 100.
Visible-message queries apply the same page bound and deduplicate shared
messages. Storage checks conversions between unsigned domain values and
SQLite's signed integer range, returning an error when a value cannot be
represented.

### Connection and concurrency model

A `Storage` handle shares one SQLite connection behind one standard mutex.
Every operation, including reads, takes this lock. Async callers schedule
storage and matching work on Tokio's blocking pool so synchronous SQLite calls
do not block runtime worker threads. The mutex serializes reads and writes
through this handle; WAL and the busy timeout do not create concurrent queries
on a single locked connection. SQLite itself still serializes writers across
connections. Maximum-size concurrent mail submissions, blocking-pool queueing,
and MIME allocations consume resources; configured input/session limits are
ceilings, not throughput guarantees. No performance guarantee is implied
without representative measurements.

## Planned storage work

A separate blob store for attachment bytes is deferred. Any future implementation
must coordinate staged blob writes with database references and safely reclaim
orphaned blobs after failed or interrupted writes. Search-specific full-text
indexing and a bounded writer queue with separate read-only connections are
also future work; the current implementation uses SQLite tables and the shared
connection described above.
