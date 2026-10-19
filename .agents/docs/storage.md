# Storage

Persistence in SandPost is split into a backend-independent contract and a concrete
backend. `sandpost-storage` defines the contract — capability traits, domain types,
request/result DTOs, and semantic errors — with no database driver and no SQL.
`sandpost-storage-sqlite` implements that contract on top of SQLx and owns the
physical schema, its forward and reverse migrations, and filter-to-SQL compilation. SQLite remains the
authoritative source of truth; Tantivy is a disposable derived search index that can be
reconstructed from SQLite.

The dependency direction is `application/services -> sandpost-storage (contract) <-
sandpost-storage-sqlite (backend)`. The application depends only on the trait object
`Arc<dyn Storage>`; backend selection happens once at the composition/bootstrap
boundary, and nothing in the contract depends on a backend.

## Contract

`sandpost-storage` defines the capability traits `UserStorage`,
`SmtpServerStorage`, `ImapStorage`, `ScopeStorage`, `ViewStorage`, `MessageStorage`,
`SearchSynchronizationStorage`, and `StorageHealth`, and composes them into the
object-safe `Storage` trait through a blanket implementation for any type that
implements every capability. No database driver type, SQL text, connection, transaction object, or
backend-specific error appears here.

- Errors are semantic `StorageError` values: `Unavailable`,
  `NotFound`, `ConstraintViolation`, `InvalidData`, `Migration`,
  `NewerSchema`, `DuplicateMessageIdentifier`, `DuplicateUserEmail`, `DuplicateSmtpUsername`,
  `DuplicateImapUsername`, `DuplicateImapFolderName`, `DuplicateInboxAddress`, `InvalidInboxAddress`,
  `PolicyVersionConflict`, `LastOwner`, `InstanceAlreadyInitialized`,
  `IntegerRange`, `BatchLimitExceeded`, and `Backend`. A backend maps its
  own driver failures into these.
- Filters are the canonical query AST; `FilterExpression` is
  `sandpost_query::Expression`, never a SQL fragment.
- Atomicity is a property of a storage operation, not of an exposed transaction object.
- The contract re-exports the shared domain types and defines the DTOs exchanged across the
  boundary: `NewUser`, `UpdateUser`, `NewSmtpAccess`, `SmtpAccessCredential`, `NewImapAccount`,
  `ImapAccountCredential`, `ImapAccountSettings`, `NewImapLocalFolder`, `ImapLocalFolderSettings`,
  `ImapMailboxState`, `ImapMembershipDefinition`, `ImapMember`, `ImapFlagChange`, `ImapCopyResult`,
  `MessageListQuery`,
  `MessageSummary`, `IndexedMessage`, `SearchOperation`, and
  `SearchSynchronization`.
- `IndexedMessage` includes identity, sequence, revision, and normalized facts only;
  attachment counts are derived from attachment records without loading their full metadata.
  Full attachment metadata remains part of ordinary message retrieval.

## Logical model

The logical model is backend-independent and is realized by the SQLite backend through one
ordered sequence of migration scripts.

- **Users.** One canonical record holds identity (`identifier`, `name`,
  `email`, `password_hash`), a `global_role` (`owner`, `admin`,
  `member`, `viewer`), a `mail_access` (`all`, `scoped`) that decides mail visibility, an optional
  `personal_filter`, and
  `created_at`/`updated_at`. There is no separate owner or credentials table and
  no boolean permission block. The bootstrap owner is a normal user whose `global_role`
  is `owner`; initialized storage must retain at least one Owner, enforced by the backend as
  `StorageError::LastOwner`, and the unique email constraint surfaces as
  `DuplicateUserEmail`. Startup is initialized iff storage contains at least one Owner;
  non-Owner users do not prevent setup. `create_first_owner` atomically refuses only if an
  Owner already exists, and may add an Owner alongside existing non-Owner users while
  preserving unique email.
- **SMTP listener configuration.** The listener address and security policies belong only to
  process configuration. Storage has no listener settings, singleton table, or read/write operations.
- **SMTP accesses.** `SmtpAccess { identifier, name, username, enabled, requires_encryption,
  allowed_mechanisms, created_at, last_used_at }` plus a password hash that only `SmtpAccessCredential`
  reads (used by authentication) ever return. `allowed_mechanisms` is a non-empty, canonically ordered
  set of `SmtpAuthenticationMechanism` (`plain`, `login`); an empty set is a `ConstraintViolation` at
  creation and in `set_smtp_access_authentication_rules`.
  Usernames are unique (`DuplicateSmtpUsername`). Accesses do not affect where mail is stored. Replacing the
  password hash is one atomic statement. `record_smtp_access_use` may skip writes within a minute of the
  stored time. The contract never receives a plaintext password.
- **Scopes.** Instance-wide hierarchical query definitions with
  identifier/parent/name/description/filter/position/policy_version.
- **Scope memberships.** `ScopeMembership { user_identifier, scope_identifier }` is
  role-less and only answers which mail subsets reach a user with scoped mail access. Assigning a
  user to an unknown scope fails with `NotFound`; deleting either side cascades the membership.
- **Views.** Optional owner: a user-owned view is private to its owner, while a null owner
  denotes a shared view. A view contains a name and a canonical filter. Views organize
  mail and never grant access. The nullable `icon` text column stores a Lucide icon
  name for Smart Folders and address inboxes. Icons follow View ownership: shared
  Views share their icon, managed by administrators; personal icons belong to their owner.
- **Mail.** One pool of messages with normalized recipient/header/attachment facts, raw MIME in
  the mail row, and search synchronization state. Mail records no destination or SMTP access.
- **Primary state.** `user_message_deletions` permanently removes a message from the
  primary user's view and every Mirror projection, while Independent references survive.
  `user_message_placements`, `user_message_copies`, `user_message_trash` and
  `user_message_flags` hold primary organization. Seen and supported flags share a
  primary modification cursor. Tombstones cannot be cleared through account restore.
- **IMAP accounts.** Credentials carry `mode` (Mirror/Independent) and an increasing
  `mode_generation`; mode transitions revoke authenticated sessions without overwriting
  dormant state. Account deletion cascades private state and may collect unreferenced
  mail. New credentials snapshot current authorized, untombstoned primary history and
  normal organization using `NewImapAccount.initial_authorization` in the creation
  transaction. New arrival triggers add Independent references while any mode is active.
- **Independent retention.** `imap_account_messages` records the physical messages
  retained by each account. Presence is independent of current authorization and mode;
  permission loss hides content but preserves internal organization. Permanent removal
  releases the account's reference without primary/peer deletion. No historical rescan
  recreates released references. Dormant references prevent premature collection.
- **Folders.** Primary Web definitions automatically project through Mirror accounts.
  The SQLite `web_folders` table stores the same nullable `icon` text column for
  manual folders. Icons are presentation metadata and survive IMAP renames without
  affecting membership, authorization or mailbox identities.
  Independent normal folders and smart definitions have no primary definition link;
  new accounts snapshot normal folders only. Legacy owner smart definitions remain
  Mirror-compatible. `/` paths form normal folder hierarchies; explicit parent renames change
  descendants transactionally while retaining identifiers. Deleting a normal folder
  trashes only otherwise-unfiled occurrences and preserves other normal copies.
- **Membership identities.** Identity is keyed by credential, mode and source. Dormant
  identities remain stored. UIDs are ascending, never reused, and each new identity
  receives increasing UIDVALIDITY. Synchronization combines canonical authorization,
  the smart definition where applicable, active-view retention and organization.
  Smart results ignore normal placement and exclude primary/account Trash. Normal
  copies share physical content, Seen and supported flags; Deleted is per occurrence.
  COPY/MOVE cannot insert into smart results. Repeated destination occurrences fail
  atomically, except cross-mailbox Trash transfers retain an existing occurrence and UID
  and report `all_new = false`. EXPUNGE removes the selected occurrence, promotes
  a surviving copy, otherwise moves final normal membership into Trash, and finally
  releases a last Trash occurrence. Independent operations stay account-local.
- **Garbage collection.** Reference release/account deletion collects a physical
  message only when every primary user tombstoned it and no Independent reference
  remains. This deliberately conservative rule retains primary mail temporarily
  outside current scopes. Cascading child facts/attachment metadata and raw MIME bytes
  disappear with the message; the existing mail-delete trigger queues search removal.
  Search rebuilding remains derived and never authorizes access. Privileged physical
  maintenance methods remain distinct from user-facing lifecycle operations.

Full behavior, protocol decisions, subscriptions, and notifications: [imap.md](imap.md).

## SQLite backend, startup, and migrations

`sandpost-storage-sqlite` keeps one ordered sequence of 47 SQL scripts (0001–0047, without gaps),
with table/index definitions and trigger groups in separate files. The `migrations` table records
applied script names and a positive integer `batch`. Every execution with pending scripts records
all of them in the same batch, one greater than the largest applied batch; fresh databases use batch
1. Opening an up-to-date database does not create a batch. There is no separate schema version,
and startup neither reads nor writes `PRAGMA user_version`. To evolve the schema, append a script to the
sequence; never edit or reorder a script after it has shipped.

Startup requires the recorded names to be an exact prefix of the checked-in sequence, with positive
nondecreasing batches in script order. It validates every table, constraint, index, and trigger
against that prefix, then applies only the remaining scripts and records their names and batch in
the same immediate transaction. A fresh database applies the
whole sequence; an existing supported database applies its missing suffix. `PRAGMA foreign_key_check`
must report no violation before commit. A failed upgrade rolls back both schema changes and history.
Unknown scripts (including scripts from a newer build), gaps, missing history in a populated database,
and schema mismatches fail startup; there is no automatic repair or reset.

The sequence includes canonical users, durable remembered sessions, SMTP access credentials and
authentication rules, scopes, Views, shared mail and normalized facts, the search outbox, and IMAP
state. Every table is created
with its current columns, constraints, and indexes, including the primary flag cursor on `users`,
account modes, folder kinds, and mode-specific mailbox identities. Owner folder definitions and
their account projection links have separate table scripts. Trigger groups and the effective-flags
view follow their dependencies; the primary deletion membership trigger is created once in 0046.
Migration 0047 creates remembered sessions keyed by the caller-computed SHA-256 token digest, binds
each to its user and verified password hash, and cascades session deletion with user deletion.
Sessions have no expiry and remain until explicitly revoked or their user is deleted.

The application has not shipped. Its pre-release baseline was consolidated: historical ALTERs,
data backfills, trigger replacements, and fingerprint invalidation scripts were removed. Scripts
were renumbered consecutively from 0001 to 0046 with matching rollback names; 0047 appends durable
remembered sessions. Databases created with the
earlier development sequence are unsupported by this baseline and must be recreated explicitly
for development; startup never rewrites their history or resets their data automatically. Once
released, schema evolution must append migrations and preserve the shipped prefix.

Supported name-only histories are validated against their original schema, then gain a batch column
inside the same upgrade transaction. Existing records become batch 1 because their original execution
boundaries were not recorded; pending scripts become batch 2. The older pre-release batch format with
a different table definition remains unsupported and is rejected without rewriting data.

Offline maintenance uses `sandpost migrate` (or `sandpost migrate up`), `sandpost migrate status`, and
`sandpost migrate rollback`. Status shows the applied-script and distinct-batch counts followed by
`batch`/script rows; it never applies pending migrations or changes legacy metadata. Rollback opens the
existing database directly, without running startup migrations, and executes only the latest batch's
matching `migrations/down/` scripts in reverse order. It atomically removes their history, validates
the resulting prefix schema and foreign keys, and commits; any error leaves schema, history and data
unchanged. Repeated rollback commands undo earlier batches. Reverting the initial batch also removes
the history table; an empty database can then be initialized again. A later startup or explicit migrate
reapplies pending scripts. Batch numbers follow the largest remaining batch, as in Laravel.

Mutating CLI maintenance holds the installation's search writer lock and refuses a running instance;
stop other database users before calling the backend's standalone maintenance methods. Rollback of
table creation removes the corresponding data. Down scripts cannot recreate information lost
by application writes. The consolidated baseline has no historical data conversions to reverse.
Keep a database backup when reverting populated schemas.

The physical schema is a SQLite implementation detail. The logical model above is what the
sandpost-storage contract guarantees to every backend, so a future backend may model it
differently as long as the observable behavior holds.

### Migration file conventions

Keep each SQL migration focused on one schema object or one data-conversion responsibility:

- A table-creation script contains one table, its constraints, and its indexes. Keep indexes with
  the table they serve rather than collecting unrelated indexes in another feature-wide script.
- A trigger script groups triggers for one table and one rule. For example, the INSERT and UPDATE
  guards that enforce a placement's account ownership belong together, separately from table creation.
- An alteration script changes one table and may include the directly related update needed to
  populate its new columns. Before the initial release, fold such changes into the table's
  creation script instead of retaining a historical ALTER or data backfill.
- Give an independently meaningful data backfill its own script with a specific purpose, such as
  preserving existing Trash memberships as message copies. Do not mix it with unrelated table or
  trigger definitions.
- Name each file after its responsibility, using the ordered numeric prefix followed by complete
  domain names: `create_imap_message_copies_table`, `create_imap_message_copies_triggers`, or
  `backfill_imap_trash_copies`. Avoid umbrella migrations that collect an entire feature's tables,
  indexes, triggers, and data conversions.
- Order scripts by dependency: referenced tables and columns must exist before dependent objects
  or backfills. Register every script in the same order in the migration catalog. Splitting files
  does not split the upgrade transaction: all missing scripts still commit or roll back together.

Pair each forward script with the same filename under `migrations/down/` and register its inverse
at the same position in the rollback catalog. Inverses must restore the preceding physical schema;
refuse a conversion when the available data cannot be reversed safely. Execution batches group history
rows, not independent schema-version sequences. Before release, unshipped scripts may be reorganized
with matching catalogs, tests, and documentation updates. Once a script has shipped, preserve its name and contents and append new migrations instead.

## Async execution

`SqliteStorage::open(path).await` initializes the file database and migrations through one SQLx
writer connection, then opens a pool of four read-only connections. `SqliteStorage::memory().await`
uses one pool and one connection shared by reads and writes, with idle timeout and maximum lifetime
disabled so the in-memory database remains alive. Both constructors are asynchronous; `close().await`
closes the shared pool or pools and waits for checked-out operations.

Each connection uses `synchronous=FULL`, foreign keys, and a five-second SQLite busy timeout. Pool
acquisition also times out after five seconds. The one-connection writer pool serializes writes; the
read-only pool allows up to four concurrent reads under WAL. This scheduling remains an implementation
detail of `sandpost-storage-sqlite` and does not cross the storage contract.

## Mail writes and reads

SMTP ingestion writes the message, normalized recipient/header/attachment facts, and
exactly one search-outbox operation in one transaction. SQLite reserves the next
AUTOINCREMENT sequence under an immediate write transaction, writes child facts before
the parent, and defers foreign-key checks until commit. The final parent insert enqueues
one complete revision; later child mutations still advance revisions through the existing
triggers. Foreign keys and triggers remain enabled. A failed transaction produces
neither a partial message nor an outbox row. The SMTP server acknowledges only after the
transaction commits. Privileged physical `delete_message` and `delete_message_at_revision`
transactionally cascade dependent facts and enqueue the corresponding index operation.
Ordinary application deletion uses `delete_user_messages`: its composed authorization filter
is evaluated against authoritative mail in one transaction, inserting only new user tombstones
and returning their newly inserted count. `None` identifiers selects all matching mail; a supplied
identifier batch of at most 256 entries restricts selection. An unknown user returns
`StorageError::NotFound`; an empty supplied batch for an existing user returns zero. Repeated
insertion is idempotent. Physical contents,
revisions, and global search-outbox state remain untouched.
Raw MIME remains in the mail row; attachment bytes are
transfer-decoded from that source when requested through `sandpost-mime`. Text charset
conversion applies to searchable/displayed bodies, never to downloaded attachment bytes.
Attachment sizes and SHA-256 hashes describe those same transfer-decoded octets; temporary
MIME recovery delimiters never enter raw storage or attachment downloads.

SQLite writes recipient, header, and attachment child facts in multi-row statements capped at
128 rows each, still inside the message transaction. List and summary-hydration reads combine
normalized recipient facts into an ordered JSON projection with one transient result row per
message; persisted tables and normalized facts are unchanged. Summary reads select preview and
metadata only and do not fetch raw MIME. Full-message and raw-MIME reads are explicit operations.

Message identity and insertion sequence are immutable SQL keys; sequence is the stable
ordering key. Lists and search pages use newest-first keyset pagination
(`sequence < before`) rather than offsets. SQL reads are bounded; search
indexing/rebuild fetches rows in bounded sequence batches, and application hydration loads
only the selected result batch. Summary reads use bounded excerpts and metadata, while
full/raw reads are explicit. Storage validates unsigned-to-SQLite integer conversions and
rejects values outside SQLite's signed integer range. Installation identity metadata is
separately stored as `instance.json` in the configured data directory; it is not a SQLite
record and does not control setup state. Storage is authoritative: an instance is initialized
iff at least one Owner exists. Setup is offered with no Owner even if non-Owner users remain
or `instance.json` exists; its atomic first-owner operation preserves those users and refuses
if another setup has already created an Owner. Metadata is written after Owner creation. A
publication failure locks setup for the current process, while a restart sees the committed
Owner and starts initialized regardless of missing or malformed metadata.

## Direct authorization

Direct message authorization must use the same composed plan as search: the user's mail
access `all` or the union of the user's assigned scopes with inherited filters, then
personal restrictions, plus requested view/query constraints. Direct reads evaluate that plan
against facts and the requested payload from one SQL snapshot. Keep this check in front
of summary, full-message, raw MIME, attachment, and event operations as applicable. Unknown
and unauthorized identifiers should not reveal existence where the API currently returns a
common not-found result. `get_user_message` and `get_user_message_metadata` exclude tombstones
in the same snapshot as their payload, but the user identifier alone does not authorize the
read; the caller must still evaluate its composed authorization plan before exposing the payload.
`list_user_messages` applies the supplied query filter, which must include the composed
authorization plan, and excludes tombstones in SQL before pagination;
`hydrate_current_user_messages` also applies the canonical filter and indexed revision in SQL,
preserving request order. Owners bypass scope restrictions but never their own deletion state.

## Search outbox and derived index

The SQL outbox is durable synchronization state, not the search index itself. Outbox writes
are part of the message transaction. The worker fetches operation pages of 64 and coalesces a
contiguous prefix of at most 1024 operations for at most 64 distinct message identifiers per
commit. Repeated header, recipient, and attachment operations replace each message only once
from current SQL state (making replay idempotent). The worker commits Tantivy and only then
acknowledges the consumed prefix in SQLite; an excluded identifier and its suffix remain queued.
A crash between index commit and acknowledgment safely replays the batch. The worker polls
SQL progress every 10 ms and waits for 50 ms without new mutations before indexing.
Continuous traffic starts indexing after at most five seconds rather than starving the
index. Remaining batches drain without another quiet-period wait; shutdown bypasses the
debounce and catches up immediately. Search and list-refresh notifications intentionally
lag committed SQL state during this window. Tantivy uses one indexing worker with a 15 MB
heap budget to avoid spreading small batches across multiple segments.

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

SQLite storage uses four pooled read-only connections and one pooled writer connection. The writer
pool serializes writes while concurrent reads use the reader pool under WAL. In-memory storage shares
a single connection for reads and writes. Search
has a separate writer gate and one installation lock. Rebuild is bounded by page/batch sizes
but consumes time and disk proportional to the corpus. No throughput guarantee is implied
without representative measurements.

Every message carries a persistent search revision assigned from the globally monotonic outbox
sequence. Indexed-fact, raw MIME, recipient, header, and attachment changes advance
the revision and enqueue synchronization in the same transaction. Delete/reinsert cannot reuse
a revision. Index reads obtain revision and normalized facts from one SQL read transaction.
`hydrate_current_messages` batch-loads lightweight summaries only for identifier/revision
pairs equal to the current mail row, with child summaries in the same snapshot; stale hits are
omitted until indexing catches up. User-facing search uses `hydrate_current_user_messages` and
refills bounded candidate pages until the visible page is full or exhausted, advancing by
indexed sequences even if an entire candidate page has physically vanished. User deletion
never removes a document from the global index.
Temporary memory indexes retain their own observed watermark
and cannot prune the durable SQL outbox.

Refresh notifications wait for a captured committed index watermark through a watch channel,
then recheck message visibility and session validity before exposing IDs. SSE `ready` also
waits for that captured watermark. SMTP acknowledgement still depends only on the SQL
transaction, and direct reads remain immediately available. Shutdown releases the installation
and Tantivy writer locks even when application/reader handles survive, allowing native restart;
closed writers reject further mutations and wake pending waits. Temporary coordinators fence
rebuilds against concurrent durable outbox pruning and retry or explicitly fail under sustained
acknowledgement churn.

Mailbox COPY targets persist for INBOX, Trash, manual and private folders. Moving an
additional COPY occurrence changes that occurrence and preserves its independent primary
placement. A duplicate destination membership is refused atomically, including same-mailbox
COPY/MOVE. Cross-mailbox Trash transfers instead coalesce an existing destination occurrence
without consuming another UID; mixed/coalesced batches omit COPYUID. Explicit subscriptions
and observed default subscriptions persist by name even after
the catalog entry disappears. Retaining observed defaults atomically respects unsubscribe
exceptions and never overwrites a concurrent UNSUBSCRIBE.
Ownership checks cover inserts and updates to placement and exposure relationships.
