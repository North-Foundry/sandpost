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
| `0006_create_mail_table.sql` | `mail`: sequence, identifier, subject, bodies, RFC message identifier, raw bytes, receipt time, and size |
| `0007_create_mail_recipients_table.sql` | `mail_recipients`: ordered envelope and header mailboxes, with role and domain |
| `0008_create_mail_headers_table.sql` | `mail_headers`: ordered header names and values |
| `0009_create_mail_scope_table.sql` | `mail_scope`: versioned scope matches, keyed by scope identifier and mail sequence |
| `0010_create_mail_attachments_table.sql` | `mail_attachments`: ordered attachment metadata and content hashes |

The baseline uses complete identifier names in its columns. `mail` contains
`sequence`, `identifier`, `subject`, `text_body`, `markup_body`,
`message_identifier`, `raw_message`, `received_at`, and `size`.
`mail_recipients` contains `mail_sequence`, `recipient_type`, `ordinal`,
`address`, and `domain`; `mail_headers` contains `mail_sequence`, `name`,
`value`, and `ordinal`; `mail_scope` contains `scope_identifier`,
`mail_sequence`, and `policy_version`; and `mail_attachments` contains
`mail_sequence`, `ordinal`, `filename`, `content_type`, `size`, and
`content_hash`. The primary keys in `scopes`, `users`, and `inboxes` are named
`identifier`; their foreign keys use `parent_identifier`, `user_identifier`,
and `scope_identifier` as appropriate. Mail relations spell out references as
`mail_sequence` and `scope_identifier`.

These ten files collectively form **schema version 1**, recorded in SQLite's
`PRAGMA user_version`. File prefixes express execution order, not separate schema
versions. All ten history entries have batch 1, including creation of the
history table. Files are embedded at compilation; deployment needs no SQL files
on disk. There is no migration framework or additional dependency.

Startup acquires a SQLite write transaction before checking the version. It
supports two states: a fresh empty version 0 database, which receives all ten
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
| `mail_parts.rs` | Relational recipient, header, and attachment writes and reads |
| `scopes.rs` | Scope persistence and policy version guards |
| `visibility.rs` | Materialization replacement and deduplicated visibility reads |
| `records.rs` | Public summaries and validated row decoding |
| `error.rs` | Storage errors |

Foreign keys, WAL, and a five-second busy timeout are enabled. One mutex protects
the shared connection, serializing reads and writes; WAL does not remove that
lock. These APIs are synchronous. The application runs database work on its
blocking pool. Additional connections should follow measured contention.

Mail facts are stored in relational columns and child rows; the database stores
no JSON-encoded message facts. `mail_recipients` preserves each mailbox's role,
ordinal, address, and domain, including duplicates across envelope and header
roles. The role values are `envelope_from`, `envelope_to`, `from`, `to`, and
`carbon_copy`. Candidate lookup indexes are role-aware and do not merge these
mailbox sets; they remain candidate indexes, not complete indexes for every
query-language field. Headers preserve the order of repeated values for each name.
`mail_attachments` stores ordered metadata and content hashes, while attachment
bytes remain in the original `raw_message` bytes in `mail`. Summary attachment
counts and detail child-row counts are computed from relational rows; an input
`attachment_count` is not persisted. Summary reads remain bounded to 100 rows
and omit body and raw-message bytes; each page loads its mailbox rows in one query.

## Verification

From the repository root:

```sh
cargo test --locked -p sandpost-storage
cargo clippy --locked -p sandpost-storage --all-targets -- -D warnings
```

`src/tests/` groups internal tests by messages, visibility, and migrations.
`tests/` exercises the exported API and serialized domain representations. The
example above also runs as a Rust documentation test.
