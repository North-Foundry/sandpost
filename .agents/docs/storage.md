# Storage

Persistence in SandPost is split into a backend-independent contract and a concrete
backend. `sandpost-storage` defines the contract — capability traits, domain types,
request/result DTOs, and semantic errors — with no database driver and no SQL.
`sandpost-storage-sqlite` implements that contract on top of rusqlite and owns the
physical schema, its forward migrations, and filter-to-SQL compilation. SQLite remains the
authoritative source of truth; Tantivy is a disposable derived search index that can be
reconstructed from SQLite.

The dependency direction is `application/services -> sandpost-storage (contract) <-
sandpost-storage-sqlite (backend)`. The application depends only on the trait object
`Arc<dyn Storage>`; backend selection happens once at the composition/bootstrap
boundary, and nothing in the contract depends on a backend.

## Contract

`sandpost-storage` defines the capability traits `UserStorage`,
`EndpointStorage`, `ScopeStorage`, `ViewStorage`, `MessageStorage`,
`SearchSynchronizationStorage`, and `StorageHealth`, and composes them into the
object-safe `Storage` trait through a blanket implementation for any type that
implements every capability. No rusqlite type, SQL text, connection, transaction object, or
backend-specific error appears here.

- Errors are semantic `StorageError` values: `Unavailable`, `Conflict`,
  `NotFound`, `ConstraintViolation`, `InvalidData`, `Migration`,
  `NewerSchema`, `DuplicateMessageIdentifier`, `DuplicateUserEmail`,
  `DuplicateInboxAddress`, `InvalidInboxAddress`, `StaleRevision`,
  `PolicyVersionConflict`, `LastOwner`, `InstanceAlreadyInitialized`,
  `IntegerRange`, `BatchLimitExceeded`, and `Backend`. A backend maps its
  own driver failures into these.
- Filters are the canonical query AST; `FilterExpression` is
  `sandpost_query::Expression`, never a SQL fragment.
- Atomicity is a property of a storage operation, not of an exposed transaction object.
- The contract re-exports the shared domain types and defines the DTOs exchanged across the
  boundary: `NewUser`, `UpdateUser`, `MessageListQuery`,
  `MessageSummary`, `IndexedMessage`, `SearchOperation`, and
  `SearchSynchronization`.

## Logical model

The logical model is backend-independent and is realized by the SQLite backend as a single
physical schema baseline.

- **Users.** One canonical record holds identity (`identifier`, `name`,
  `email`, `password_hash`), a `global_role` (`owner`, `admin`,
  `member`), an optional `personal_filter`, and
  `created_at`/`updated_at`. There is no separate owner or credentials table and
  no boolean permission block. The bootstrap owner is a normal user whose `global_role`
  is `owner`; the "at least one owner" invariant is enforced by the backend as
  `StorageError::LastOwner`, and the unique email constraint surfaces as
  `DuplicateUserEmail`.
- **Endpoints.** SMTP endpoints with stable identifiers, unchanged in shape.
- **Endpoint memberships.** `EndpointMembership { user_identifier, endpoint_identifier,
  role: EndpointRole, mail_access: MailAccess }`, keyed by
  `(user_identifier, endpoint_identifier)`. The endpoint role (`admin`,
  `member`, `viewer`) carries endpoint-local administrative authority; mail access
  (`all`, `scoped`) carries mail visibility.
- **Scopes.** Endpoint-associated, hierarchical query definitions with
  identifier/parent/name/description/filter/position/policy_version, unchanged in shape.
- **Scope memberships.** `ScopeMembership { user_identifier, scope_identifier }` is
  role-less and only answers which mail subsets reach a user. A scope membership requires an
  endpoint membership for the scope's endpoint; the storage API rejects an orphaned
  assignment with `ConstraintViolation`.
- **Views.** Optional owner: a user-owned view is private to its owner, while a null owner
  denotes a shared view. A view contains an endpoint and a canonical filter. Views organize
  mail and never grant access.
- **Mail.** Messages, normalized recipient/header/attachment facts, raw MIME in the mail row,
  and search synchronization state.

## SQLite backend, startup, and migrations

`PRAGMA user_version` is schema version 1 under `sandpost-storage-sqlite`. The schema is a
single flattened baseline applied from ordered per-operation scripts: SandPost has not been
deployed, so a fresh database is created directly at the current physical schema and there is no
incremental migration history to replay.

Fresh databases receive the baseline atomically and record it in the `migrations` history
table. Reopening validates that the stored `user_version` equals the baseline and that every
table, constraint, index, and trigger matches it. Negative, newer, unversioned-populated,
missing-history, mismatched-history, or structurally invalid databases fail startup; older
schemas are rejected rather than migrated and there is no schema repair.

The physical schema is a SQLite implementation detail. The logical model above is what the
sandpost-storage contract guarantees to every backend, so a future backend may model it
differently as long as the observable behavior holds.

## Async execution

SQLite itself is synchronous, but the backend exposes only async trait methods. Every call
schedules its blocking work through `tokio::task::spawn_blocking` behind one
mutex-protected connection, so async callers never block runtime worker threads. The
connection enables foreign keys, requests WAL mode (retrying busy errors for up to five
seconds), and uses a five-second busy timeout. That internal synchronization is an
implementation detail and never crosses the crate boundary: one `SqliteStorage` handle
serializes every operation, including reads, and SQLite serializes writes across separate
connections. WAL does not make a single handle concurrently queryable.

## Mail writes and reads

SMTP ingestion writes the message, normalized recipient/header/attachment facts, and
transactional search-outbox operations in one transaction. A failed transaction produces
neither a partial message nor an outbox row. The SMTP server acknowledges only after the
transaction commits. Message deletion transactionally cascades dependent facts and enqueues
the corresponding index operation. Raw MIME remains in the mail row; attachment bytes are
decoded from that source when requested.

Message identity and insertion sequence are immutable SQL keys; sequence is the stable
ordering key. Lists and search pages use newest-first keyset pagination
(`sequence < before`) rather than offsets. SQL reads are bounded; search
indexing/rebuild fetches rows in bounded sequence batches, and application hydration loads
only the selected result batch. Summary reads use bounded excerpts and metadata, while
full/raw reads are explicit. Storage validates unsigned-to-SQLite integer conversions and
rejects values outside SQLite's signed integer range. Instance identity/bootstrap metadata is
separately stored as `instance.json` in the configured data directory; it is not a
SQLite record.

## Direct authorization

Direct message authorization must use the same composed plan as search: per endpoint, mail
access `all` or the union of the user's assigned scopes with inherited filters, then
personal restrictions, plus requested view/query constraints. Direct reads evaluate that plan
against endpoint, facts, and requested payload from one SQL snapshot. Keep this check in front
of summary, full-message, raw MIME, attachment, and event operations as applicable. Unknown
and unauthorized identifiers should not reveal existence where the API currently returns a
common not-found result.

## Search outbox and derived index

The SQL outbox is durable synchronization state, not the search index itself. Outbox writes
are part of the message transaction. The worker reads operations in batches of 64, updates
Tantivy from current SQL state (making replay idempotent), commits the index, and only then
acknowledges the operations in SQLite. A crash between index commit and acknowledgment safely
replays the batch. The worker polls on a 100 ms interval; search can lag committed SQL state
until it catches up.

The installation has a `CURRENT` generation pointer and a process-exclusive writer lock.
The active generation is durable, but disposable: missing, corrupt, or behind state is rebuilt
from SQL. Rebuild takes a bounded baseline through a captured sequence, replays retained
insert/delete operations through a cutoff while ingestion continues, then publishes the
generation atomically and acknowledges the replayed watermark. Later outbox work remains for
the worker. Reader handles keep retired generations alive until in-flight queries finish.

Search status reports SQL's latest mutation sequence, acknowledged/indexed sequence, and
pending operation count. The CLI `search status` command reads this SQL state directly and
does not acquire the search writer lock. CLI `search rebuild` opens the coordinator and
must acquire the same process-exclusive lock used by the runtime, so it cannot run while a live
runtime owns that lock. HTTP status and rebuild require an administrator (owner or admin) and
use the running coordinator; rebuild is serialized through that coordinator's writer gate
without attempting to acquire a second installation lock.

## Concurrency

The shared storage handle serializes every operation, including reads. Async entry points
schedule synchronous SQLite work away from Tokio worker threads. SQLite serializes writes
across separate connections; WAL does not make a single handle concurrently queryable. Search
has a separate writer gate and one installation lock. Rebuild is bounded by page/batch sizes
but consumes time and disk proportional to the corpus. No throughput guarantee is implied
without representative measurements.

Every message carries a persistent search revision assigned from the globally monotonic outbox
sequence. Indexed-fact, endpoint, raw MIME, recipient, header, and attachment changes advance
the revision and enqueue synchronization in the same transaction. Delete/reinsert cannot reuse
a revision. Index reads obtain revision and normalized facts from one SQL read transaction.
`hydrate_current_messages` batch-loads lightweight summaries only for identifier/revision
pairs equal to the current mail row, with child summaries in the same snapshot; stale hits are
omitted until indexing catches up. Temporary memory indexes retain their own observed watermark
and cannot prune the durable SQL outbox.

Refresh notifications wait for a captured committed index watermark through a watch channel,
then recheck message visibility and session validity before exposing IDs. SSE `ready` also
waits for that captured watermark. SMTP acknowledgement still depends only on the SQL
transaction, and direct reads remain immediately available. Shutdown releases the installation
and Tantivy writer locks even when application/reader handles survive, allowing native restart;
closed writers reject further mutations and wake pending waits. Temporary coordinators fence
rebuilds against concurrent durable outbox pruning and retry or explicitly fail under sustained
acknowledgement churn.
