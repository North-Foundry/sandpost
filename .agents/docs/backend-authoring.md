# Implementing a SandPost storage backend

SandPost persistence is replaceable. The application depends only on the backend-neutral
contract in `sandpost-storage`; the built-in SQLite backend is one implementation of it. A
third party can implement another backend (PostgreSQL, MySQL, CockroachDB, or a non-relational
engine) without changing application or business logic.

This guide states the contract a backend must satisfy. Behaviour, not method signatures, is what
the application relies on.

## 1. Which crates to depend on

- `sandpost-core` — domain identifiers and facts (users, roles, memberships, scopes, views,
  messages). Backend-neutral.
- `sandpost-storage` — the capability traits, request/result DTOs, and `StorageError`.
- `sandpost-query` — the canonical filter AST accepted by message queries.

Do not depend on `sandpost-storage-sqlite`; it exists only to provide the default backend.

## 2. Which traits to implement

Implement each capability trait for one concrete type, then expose that type as
`Arc<dyn sandpost_storage::Storage>`:

- `UserStorage` — canonical users, credentials, global roles, bootstrap.
- `EndpointStorage` — endpoints and per-user endpoint memberships.
- `ScopeStorage` — hierarchical scopes and role-less scope memberships.
- `ViewStorage` — personal and shared views, plus the legacy recipient-view helper.
- `MessageStorage` — ingestion, retrieval, listing, deletion, index projection.
- `SearchSynchronizationStorage` — the durable search-outbox protocol.
- `StorageHealth` — liveness probing.

`Storage` is automatically implemented for any type implementing all of the above and
`Send + Sync`; there is nothing to implement for it directly. The type must be `Send + Sync`
because it is shared across request handlers and tasks.

## 3. Concurrency guarantees

- Every method must be safe for concurrent callers. The application holds a single
  `Arc<dyn Storage>` and calls it from many tasks.
- Blocking work must not run on async executor threads. If the underlying driver is synchronous
  (as SQLite is), schedule it on a blocking pool (`tokio::task::spawn_blocking`) behind whatever
  internal synchronization the driver needs. Internal locks or connection pools are fine as long
  as they never surface through the trait methods.
- Methods are async and their futures must be `Send`.

## 4. Transaction and atomicity expectations

The contract never exposes a transaction object. Atomicity is a property of individual
operations:

- `insert_message` writes the message and all normalized child facts (recipients, headers,
  attachments) atomically, and atomically with its search-outbox operation.
- `delete_message` and `delete_message_at_revision` remove the message, its child facts, and the
  corresponding search-outbox operation atomically.
- `create_first_owner` atomically refuses if any user already exists.
- `update_user`/`delete_user` atomically preserve the "at least one owner" invariant.
- `remove_endpoint_membership` atomically removes the membership and every dependent scope
  membership for that endpoint.
- `acknowledge_search_operations` advances the acknowledged watermark and prunes the
  acknowledged prefix atomically.

If a higher-level operation needs multiple storage calls to be consistent, prefer adding a
single atomic storage operation over exposing a transaction handle to the caller.

## 5. Message ordering semantics

- Every message has a stable unique identifier and an insertion sequence.
- Sequence is monotonic and is the ordering key. Listings return newest first.
- Keyset pagination uses an exclusive upper bound (`before`), never offsets.
- `index_messages(after, through, limit)` returns records in ascending sequence order, strictly
  after `after` and at most through an inclusive `through`, bounded by `limit`.
- `hydrate_messages` preserves the caller's identifier order and omits missing identifiers.
- `hydrate_current_messages` returns a summary only when the current revision equals the
  requested revision, preserving request order and omitting stale or deleted messages.

## 6. Revision semantics

- Each message carries a durable search revision drawn from a globally increasing operation
  sequence. A committed message mutation and its search operation advance the revision in the
  same transaction.
- Delete/reinsert never reuses a revision.
- `delete_message_at_revision` deletes only when the stored revision equals the requested
  revision, returning `false` for a stale or missing message.
- Index reads (`indexed_messages`) return the current revision and normalized facts from one
  read snapshot.

## 7. Search synchronization semantics

The search outbox is durable synchronization state, not the search index. A backend must provide
the observable behaviour currently provided by SQLite triggers:

- A committed message mutation and its outbox mutation are atomic from SandPost's point of view.
- Outbox operations are ordered by a globally increasing sequence and are replayable; replaying
  an acknowledged-but-not-yet-pruned prefix is idempotent.
- `search_status` reports the latest mutation sequence, the acknowledged/indexed sequence, and
  the count of queued operations.
- `search_batch(after, limit)` returns synchronization state and a bounded batch of pending
  operations from one snapshot.
- `acknowledge_search_operations(through)` advances the acknowledged watermark to the smaller
  of the current latest sequence and `through`, and prunes acknowledged operations.

A non-relational backend can implement this with an append-only log plus an acknowledgment
marker rather than triggers.

## 8. Error mapping

Translate driver errors into `StorageError` inside the backend; never expose a concrete driver
type. Useful variants:

- `Unavailable` — the backend cannot currently serve the request (closed writer, lock, outage).
- `Conflict` — a uniqueness conflict the caller can resolve.
- `NotFound` — a referenced record does not exist.
- `ConstraintViolation(String)` — a referential or domain rule rejected the request
  (for example, a scope membership without an endpoint membership).
- `InvalidData(String)` — stored data or input was structurally invalid.
- `Migration(String)` — schema creation or forward migration failed.
- `NewerSchema(i64)` — the stored schema version is newer than the backend supports.
- `DuplicateMessageIdentifier`, `DuplicateUserEmail`, `DuplicateInboxAddress`,
  `InvalidInboxAddress`, `StaleRevision`, `PolicyVersionConflict`, `LastOwner`,
  `InstanceAlreadyInitialized`, `IntegerRange`, `BatchLimitExceeded`.

Preserve a backend-specific source error through the opaque `Backend(Box<dyn Error + Send +
Sync>)` variant for logs and diagnostics without exposing its type to callers.

## 9. Migration and schema ownership

- Migrations and the physical schema belong to the backend. SandPost does not ship a portable
  migration directory; the logical model is shared, the physical schema is not.
- The backend must validate its own schema version on open and migrate supported older databases
  forward, preserving identities and mail.
- The logical model (see `.agents/docs/storage.md`) is: users, endpoints, endpoint memberships,
  scopes, scope memberships, views, mail + message facts, and the search outbox. A backend may
  model these differently as long as the observable contract holds.

## 10. Filter and query behaviour

- `MessageListQuery.filter` carries the canonical `FilterExpression` (the shared query AST),
  never a SQL string. Compile it inside the backend.
- Evaluate the AST with the query language's two-valued semantics: a missing field value makes a
  predicate false rather than unknown, and mailbox and content string comparisons fold ASCII
  case. `list_messages` results must match `sandpost_query::Expression::evaluate` on the same
  facts.
- Use the existing query language as the source of truth; do not invent operators.

## 11. Required invariants

- There is always at least one owner. Demoting or deleting the final owner fails with
  `LastOwner`.
- A scope membership requires an endpoint membership for the scope's endpoint; otherwise the
  assignment fails with `ConstraintViolation`.
- An endpoint membership with `MailAccess::Scoped` and zero assigned scopes yields zero mail
  visibility (fail closed), never full access.
- Deleting a user cascades their memberships and owned personal views.

## 12. Testing your backend

Run the reusable conformance suite in `sandpost-storage-conformance` against your implementation.
It is self-contained and independent of SQLite. A minimal harness looks like:

    let storage: Arc<dyn Storage> = Arc::new(MyStorage::open(config).await?);
    sandpost_storage_conformance::run_all(storage.as_ref()).await?;

The suite covers user CRUD and roles, the owner invariant, bootstrap, endpoint memberships and
mail access, scope membership invariants, views, message ingestion/retrieval/ordering/pagination,
filtered listing equivalence with the canonical evaluator, deletion and revisions, and the search
outbox protocol. Passing it is the expected bar for a production-ready backend.

