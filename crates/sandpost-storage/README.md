# sandpost-storage

SQLite persistence for Sand Post messages, scopes, and materialized visibility.
This crate stores domain values from `sandpost-core`; it does not parse mail,
evaluate query expressions, or decide which scopes a user may access.

## Public API

`Storage::open(path)` opens or creates a database and applies migrations.
`Storage::memory()` provides the same schema in memory. Cloned handles share one
connection. `health()` checks that the connection can execute a query.

```rust
use sandpost_core::{Message, MessageFacts, MessageIdentifier};
use sandpost_storage::Storage;

let storage = Storage::memory()?;
let message = Message {
    identifier: MessageIdentifier::new(),
    facts: MessageFacts {
        subject: "Build completed".into(),
        ..MessageFacts::default()
    },
    raw_message: b"Subject: Build completed\r\n\r\nDone.".to_vec(),
    attachments: vec![],
};

let sequence = storage.insert_message(&message, &[])?;
let page = storage.list_messages(None, 20)?;
assert_eq!(page[0].sequence, sequence);
assert_eq!(page[0].subject, "Build completed");
assert_eq!(storage.get_message(message.identifier)?.unwrap().raw_message,
           message.raw_message);
# Ok::<(), sandpost_storage::StorageError>(())
```

| Method | Responsibility |
| --- | --- |
| `insert_message` | Insert a message, recipient/header projections, and supplied scope matches in one transaction. Reject duplicate identifiers and stale scope policies. |
| `list_messages` | Read lightweight summaries, ordered by descending sequence. |
| `get_message` | Read facts, attachment metadata, and original raw message bytes. Return `None` for an unknown identifier. |
| `get_message_metadata` | Read facts and attachment metadata without loading raw message bytes. |
| `save_scope` | Persist a scope, requiring a higher policy version when its filter or parent changes. |
| `load_scopes` | Read scopes ordered by position, then identifier. |
| `replace_scope_matches` | Replace a scope's complete materialization atomically, provided its policy version is current. |
| `list_visible_messages` | Read the deduplicated union of supplied scopes, including only matches with current policy versions. |

Both list methods cap pages at 100 messages. A zero limit returns no messages.
The optional `before` cursor is exclusive: pass the oldest sequence from the
previous page to continue. A summary contains identifiers, subject, From/To
mailboxes, receipt time, size, and attachment count; reading a page does not
load bodies or raw mail. SQLite stores signed integers, so unsigned values
outside its range return `StorageError::IntegerRange`.

Scope matching and authorization belong to the caller. Passing scopes to
`list_visible_messages` does not establish that the caller may access them.
Changing a policy version makes old matches invisible until they are rebuilt.
Failed inserts or replacements roll back all associated changes.

## Table migrations and schema version

Each ordered SQL file creates one table and its indexes:

| File in `migrations/` | Table |
| --- | --- |
| `0001_create_migrations_table.sql` | `migrations`: applied file names and batches |
| `0002_create_scopes_table.sql` | `scopes`: hierarchy, filters, and policy versions |
| `0003_create_users_table.sql` | `users`: domain user records |
| `0004_create_memberships_table.sql` | `memberships`: user/scope roles |
| `0005_create_inboxes_table.sql` | `inboxes`: user-owned named filters |
| `0006_create_messages_table.sql` | `messages`: sequence, facts, raw bytes, and summary columns |
| `0007_create_message_recipients_table.sql` | `message_recipients`: mailbox projections |
| `0008_create_message_headers_table.sql` | `message_headers`: ordered header values |
| `0009_create_message_scope_table.sql` | `message_scope`: versioned scope matches |

These nine files collectively form **schema version 1**, recorded in SQLite's
`PRAGMA user_version`. File prefixes express execution order, not separate schema
versions. All nine history entries have batch 1, including creation of the
history table. Files are embedded at compilation; deployment needs no SQL files
on disk. There is no migration framework or additional dependency.

Startup acquires a SQLite write transaction before checking the version. It
supports two states: a fresh empty version 0 database, which receives all nine
baseline migrations atomically, and version 1 with matching migration history
and schema, which opens without rerunning creation files. It rejects negative
or newer versions, a populated version 0 database, and version 1 without
complete migration history. Baseline schema validation normalizes SQLite schema
objects and verifies the expected tables and indexes; an empty-schema check
distinguishes a fresh database from an unsupported populated one.

The users, memberships, and inboxes tables exist in the database, but this crate
does not yet provide persistence methods for those domain records.

## Module map

| Module | Responsibility |
| --- | --- |
| `lib.rs` | Module declarations and public exports |
| `connection.rs` | Shared connection, SQLite settings, startup, and health |
| `migrations.rs` | Baseline order, transaction, version, and migration history |
| `schema.rs` | Baseline schema validation and empty-database detection |
| `messages.rs` | Message ingestion and retrieval |
| `message_indexes.rs` | Recipient and header projections during ingestion |
| `scopes.rs` | Scope persistence and policy version guards |
| `visibility.rs` | Materialization replacement and deduplicated visibility reads |
| `records.rs` | Public summaries and validated row decoding |
| `error.rs` | Storage errors |

Foreign keys, WAL, and a five-second busy timeout are enabled. One mutex protects
the shared connection, serializing reads and writes; WAL does not remove that
lock. These APIs are synchronous. The application runs database work on its
blocking pool. Additional connections should follow measured contention.

Recipient indexes merge envelope recipients, To, and Cc. The sender-domain
projection prefers the envelope sender, then From. These projections are useful
candidate indexes, not a complete index for every query-language field. Raw mail
contains attachment bytes; the separate attachments field stores metadata.

## Verification

From the repository root:

```sh
cargo test --locked -p sandpost-storage
cargo clippy --locked -p sandpost-storage --all-targets -- -D warnings
```

`src/tests/` groups internal tests by messages, visibility, and migrations.
`tests/` exercises the exported API and serialized domain representations. The
example above also runs as a Rust documentation test.
