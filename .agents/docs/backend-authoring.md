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

- `UserStorage` — canonical users, credentials, global roles, mail access, bootstrap.
- `SmtpServerStorage` — SMTP access credentials (hashes only) and per-access authentication rules
  (TLS requirement, non-empty set of allowed mechanisms). Listener configuration is runtime-owned.
- `ImapStorage` — user-owned IMAP accounts (hashes only), linked Views, IMAP-local folders, mailbox
  identities with UIDVALIDITY and UID counters, materialized UID-keyed membership, flags,
  exclusions, and subscriptions (see section 11.1).
- `ScopeStorage` — hierarchical scopes and role-less scope memberships.
- `ViewStorage` — personal and shared views, plus the legacy recipient-view helper.
- `MessageStorage` — physical ingestion/retrieval, user-scoped reads and logical deletion,
  privileged physical deletion, and global index projection.
- `SearchSynchronizationStorage` — the durable search-outbox protocol.
- `StorageHealth` — liveness probing.

`Storage` is automatically implemented for any type implementing all of the above and
`Send + Sync`; there is nothing to implement for it directly. The type must be `Send + Sync`
because it is shared across request handlers and tasks.

## 3. Concurrency guarantees

- Every method must be safe for concurrent callers. The application holds a single
  `Arc<dyn Storage>` and calls it from many tasks.
- Blocking work must not run on async executor threads. If the underlying driver is synchronous,
  schedule it on a blocking pool behind whatever internal synchronization it needs. The current
  SQLite backend uses SQLx async pools: four read-only connections and one writer, with one shared
  connection for in-memory storage. Internal pools or locks are fine as long as they never surface
  through trait methods.
- Methods are async and their futures must be `Send`.

## 4. Transaction and atomicity expectations

The contract never exposes a transaction object. Atomicity is a property of individual
operations:

- `insert_message` writes the message and all normalized child facts (recipients, headers,
  attachments) atomically, and atomically with its search-outbox operation.
- `delete_message` and `delete_message_at_revision` remove the message, its child facts, and the
  corresponding search-outbox operation atomically.
- `delete_user_messages` inserts primary tombstones atomically under current authorization;
  Mirror memberships disappear, while Independent references survive. Repeated calls
  count only new tombstones. Physical collection is allowed only after every primary
  user and Independent reference releases the content; child facts and search-outbox
  deletion must be atomic with physical removal.
- `create_first_owner` atomically refuses if an Owner already exists. It may create the first
  Owner when non-Owner users remain, preserving those users and enforcing unique email.
- `update_user`/`delete_user` atomically preserve the "at least one owner" invariant.
- `acknowledge_search_operations` advances the acknowledged watermark and prunes the
  acknowledged prefix atomically.
- `synchronize_imap_mailbox`, `reconcile_imap_mailboxes`, `store_imap_flags`,
  `expunge_imap_members`, and `copy_imap_members` are each atomic, and concurrent calls for the
  same mailbox must never assign a UID twice.

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

User-scoped full/metadata reads exclude tombstones in the same snapshot as their payload.
`list_user_messages` excludes tombstones before pagination. `hydrate_current_user_messages`
also checks the supplied canonical authorization/filter expression and indexed revision.
Owners' deletion state is enforced like every other user's. Tombstones survive scope changes,
IMAP exclusion restore, and search rebuilding. Physical absence, denied authorization, and
user deletion remain distinct storage concepts; user endpoints may map them to not-found.

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
- `NotFound` — a referenced record does not exist.
- `ConstraintViolation(String)` — a referential or domain rule rejected the request
  (for example, a scope membership for a user that does not exist).
- `InvalidData(String)` — stored data or input was structurally invalid.
- `Migration(String)` — schema creation or forward migration failed.
- `NewerSchema(i64)` — available to backends using a numeric schema version. SQLite instead rejects
  unknown migration names with `InvalidData`.
- `DuplicateMessageIdentifier`, `DuplicateUserEmail`, `DuplicateSmtpUsername`,
  `DuplicateImapUsername`, `DuplicateImapFolderName`, `DuplicateInboxAddress`,
  `InvalidInboxAddress`, `PolicyVersionConflict`, `LastOwner`,
  `InstanceAlreadyInitialized`, `IntegerRange`, `BatchLimitExceeded`.

Preserve a backend-specific source error through the opaque `Backend(Box<dyn Error + Send +
Sync>)` variant for logs and diagnostics without exposing its type to callers.

In the SQLite backend, generic SQLite constraint failures map to `ConstraintViolation`. Operations
that identify a known duplicate through a caller-facing precheck return the corresponding dedicated
duplicate error; callers should not depend on driver-specific constraint names.

## 9. Migration and schema ownership

- Migrations and the physical schema belong to the backend. SandPost does not ship a portable
  migration directory; the logical model is shared, the physical schema is not.
- The backend must validate its own migration state on open and migrate supported older databases
  forward, preserving identities and mail. SQLite uses an applied script-name prefix, without a separate
  schema version; other backends may choose their own history representation.
- The logical model (see `.agents/docs/storage.md`) is: users (with global role and mail
  access), SMTP accesses, IMAP accounts with their configuration,
  mailbox identities, membership, and flags, scopes, scope memberships, views, one pool of
  mail + message facts, user-message deletions, and the search outbox. A backend may
  model these differently as long as the observable contract holds.

For SQLite SQL changes, follow the [migration file conventions](storage.md#migration-file-conventions):
one table with its indexes, separate trigger groups, and focused data backfills. Keep dependency order
and atomic upgrades; do not introduce feature-wide migration bundles or a second schema-version scheme.
Before the initial release, incorporate new columns into their table-creation scripts and remove
obsolete development data conversions, updating both catalogs, tests, and documentation together.

## 10. Filter and query behaviour

- `MessageListQuery.filter` carries the canonical `FilterExpression` (the shared query AST),
  never a SQL string. Compile it inside the backend.
- Evaluate the AST with the query language's two-valued semantics: a missing field value makes a
  predicate false rather than unknown, and mailbox and content string comparisons fold ASCII
  case. `list_messages` results must match `sandpost_query::Expression::evaluate` on the same
  facts.
- Use the existing query language as the source of truth; do not invent operators.

## 11. Required invariants

- Initialized storage always has at least one Owner. A pre-setup store may contain non-Owner
  users without an Owner; after initialization, demoting or deleting the final Owner fails with
  `LastOwner`.
- Startup state is based on storage: the instance is initialized iff at least one Owner exists.
  With no Owner, setup is allowed even when non-Owner users exist. Backends must make
  `create_first_owner` atomic so concurrent setup attempts cannot create competing first Owners.
- Assigning a user to a scope that does not exist fails with `NotFound`.
- A user with `MailAccess::Scoped` and zero assigned scopes yields zero mail visibility (fail
  closed), never full access.
- Deleting a user cascades their scope memberships and owned personal views; deleting a scope
  cascades its memberships.
- Remembered sessions are keyed by the caller-computed SHA-256 token digest and store the user
  identifier plus the verified password hash bound at session creation. They do not expire in
  storage, are explicitly revocable, and are removed when their user is deleted.

## 11.1 IMAP state

Implement the complete view ownership, mailbox identity and lifecycle contract in
[imap.md](imap.md) and [storage.md](storage.md):

- Mirror shares primary folders, memberships, flags, Trash and deletions. Independent
  owns durable private state bounded by the current owner's permissions.
- Account creation snapshots current authorized primary history and organization,
  excluding primary tombstones and smart definitions. Future arrivals add dormant
  Independent references too. Mode switches preserve both state sets and invalidate
  old sessions through a monotonic generation.
- Mailbox identities include mode; dormant identity and ascending UID state survive
  toggles. Reconciliation returns sources in request order, never reuses a UID, and
  verifies current authorization independently of retained references.
- Seen and supported flags share one physical message state within the view; Deleted
  is mailbox-occurrence local. Web Seen and Mirror flags share primary storage/cursors.
- Smart definitions are primary-owned in Mirror and account-owned in Independent.
  Results ignore normal placement, exclude Trash, and permit only Seen updates over
  IMAP. Smart membership changes are refused atomically.
- COPY preserves source and shares content, MOVE changes only source occurrence/view,
  and duplicate destination occurrences are refused except cross-mailbox Trash transfers.
  Those retain the existing occurrence/UID and return `all_new = false`; fresh members
  in the same transaction still receive new UIDs. EXPUNGE removes flagged occurrences;
  a surviving copy is promoted, a final normal occurrence enters Trash, and expunging
  final Trash releases the view. Return source UIDs; Independent never tombstones primary.
- Normal-folder deletion trashes otherwise-unfiled occurrences and preserves copies.
  Hierarchy renames preserve identifiers and descendants atomically.
- Credential deletion cascades only private state; final-reference collection removes
  raw content, facts/attachment metadata, memberships and derived search state safely.
- Retained subscription names and explicit unsubscribe exceptions survive catalog
  changes. Observing defaults must not undo concurrent unsubscribe.

Legacy linked/exposure fields remain compatibility data; they do not authorize messages
or provide live primary organization inheritance to Independent accounts.

## 12. Testing your backend

Run the reusable conformance suite in `sandpost-storage-conformance` against your implementation.
It is self-contained and independent of SQLite. A minimal harness looks like:

    let storage: Arc<dyn Storage> = Arc::new(MyStorage::open(config).await?);
    sandpost_storage_conformance::run_all(storage.as_ref()).await?;

The suite covers user CRUD, roles, and mail access, the owner invariant, bootstrap, SMTP
accesses, IMAP accounts, configuration, mailbox identities, membership
synchronization, flags, and expunges, scope membership invariants, views, message ingestion/retrieval/ordering/pagination,
filtered listing equivalence with the canonical evaluator, deletion and revisions, and the search
outbox protocol. Passing it is the expected bar for a production-ready backend.

