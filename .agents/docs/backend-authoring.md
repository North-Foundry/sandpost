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
- `SmtpServerStorage` — the global SMTP server address and SMTP access credentials (hashes only)
  with their per-access authentication rules (TLS requirement, non-empty set of allowed mechanisms).
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
- `delete_user_messages` evaluates the composed authorization filter and optional identifier
  restriction atomically, inserts only new user-message deletion relationships, and removes
  their memberships from every IMAP account belonging to that user. Return the newly inserted
  count; repeated calls are idempotent. Preserve physical contents, revisions, and global
  search-outbox state. Physical deletion methods are for explicit privileged maintenance.
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
  access), the global SMTP server and SMTP accesses, IMAP accounts with their configuration,
  mailbox identities, membership, and flags, scopes, scope memberships, views, one pool of
  mail + message facts, user-message deletions, and the search outbox. A backend may
  model these differently as long as the observable contract holds.

For SQLite SQL changes, follow the [migration file conventions](storage.md#migration-file-conventions):
one table with its indexes, separate trigger groups, and focused data backfills. Keep dependency order
and atomic upgrades; do not introduce feature-wide migration bundles or a second schema-version scheme.

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

## 11.1 IMAP state

- `create_imap_account` fails with `DuplicateImapUsername` for a taken username and `NotFound`
  for an unknown owner. Deleting the owner deletes its accounts and everything below them;
  deleting an account never deletes mail. Only `ImapAccountCredential` reads return the hash.
- `set_imap_linked_views` replaces the selection atomically and fails with `NotFound` for an
  unknown View; deleting a View removes it from every selection and deletes its mailbox
  identities. Folder definitions belong to the Sandpost user and appear in every credential of
  that user. The existing account-scoped folder methods manage those shared definitions through
  account-specific projections, preserving compatibility identifiers. New folder names are
  unique per owner (`DuplicateImapFolderName`); preserved legacy duplicates receive deterministic
  catalog names. Edits/deletion affect every projection, deleting an account retains definitions,
  and newly created accounts discover them. Definitions and mail never belong to credentials.
- Every mailbox identity receives a UIDVALIDITY greater than every value handed out before.
  `reconcile_imap_mailboxes` returns identities in the order of the given sources, creating
  missing ones and deleting unlisted ones with their membership.
- `synchronize_imap_mailbox` evaluates the supplied canonical filter with the query language's
  exact semantics (the same as `list_messages`). After a fingerprint change it removes members
  that no longer match. Dynamic mailboxes (all but Trash) then gain every matching message that is
  neither a member nor excluded, in ascending message sequence order, with UIDs from `uid_next`.
  Existing members keep their UIDs; UIDs are never reused. User tombstones always exclude
  membership, including after unchanged fingerprints or restoring mailbox exclusions.
- Flags other than `deleted` are per account and message; `deleted` is per membership. Every flag
  change advances the account cursor reported by `imap_flag_changes`.
- Subscription names belong to the account and survive mailbox deletion or renaming.
  `list_imap_subscribed` returns retained explicit and observed default names, including absent
  mailboxes; `list_imap_unsubscribed` returns explicit exceptions. `retain_imap_subscriptions`
  inserts missing observed defaults atomically and idempotently only when no unsubscribe
  exception exists. It must never undo a concurrent `set_imap_subscription(..., false)`.
- `expunge_imap_members` records owner-level tombstones for flagged selected messages and
  atomically removes them from every mailbox of every account belonging to that owner. Return
  source UIDs for protocol responses. MOVE changes account placement,
  never user tombstones. `clear_imap_exclusions` forgets mailbox exclusions and forces full
  reconciliation, but never clears user deletions. Synchronization always excludes owner
  tombstones, including with unchanged fingerprints. `copy_imap_members` refuses every smart-folder insertion. Manual/private insertion checks
  current authorization, and a refused operation changes nothing.

### Manual organization contract

`create_web_folder` and `update_web_folder` validate every exposure belongs to the owner and
replace projections atomically. `create_imap_private_folder` affects only one account.
`place_user_messages` checks the canonical authorization and tombstones in the write transaction,
accepts at most 256 identifiers, and never changes account overrides. `list_web_mailbox_messages`
selects manual placement or the unfiled baseline before keyset pagination.
Reconciliation uses account override > exposed Web placement > smart/general baseline and always
applies authorization. Additional manual COPY memberships preserve sources. Target deletion or
exposure removal clears only associated placement and forces fallback reconciliation; scope loss
does not discard overrides. Tombstones remain independent. Copies into a destination already
containing a message are refused rather than reporting success with a reused UID.

## 12. Testing your backend

Run the reusable conformance suite in `sandpost-storage-conformance` against your implementation.
It is self-contained and independent of SQLite. A minimal harness looks like:

    let storage: Arc<dyn Storage> = Arc::new(MyStorage::open(config).await?);
    sandpost_storage_conformance::run_all(storage.as_ref()).await?;

The suite covers user CRUD, roles, and mail access, the owner invariant, bootstrap, the SMTP
server and accesses, IMAP accounts, configuration, mailbox identities, membership
synchronization, flags, and expunges, scope membership invariants, views, message ingestion/retrieval/ordering/pagination,
filtered listing equivalence with the canonical evaluator, deletion and revisions, and the search
outbox protocol. Passing it is the expected bar for a production-ready backend.

