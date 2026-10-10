# sandpost-storage-sqlite

The built-in SQLite storage backend for SandPost.

It implements every capability trait from sandpost-storage on top of rusqlite and owns the
physical SQLite schema. SQLite stays the default backend, but it is
now just one possible implementation of the backend-neutral contract: no rusqlite type, SQL
string, connection, transaction, or driver error crosses the crate boundary.

## Async execution

SQLite itself is synchronous. Every trait method schedules its blocking work through
tokio::task::spawn_blocking behind a single mutex-protected connection, so async callers never
block runtime worker threads. That internal synchronization is an implementation detail and is
not observable through the storage contract.

## Layout

- migrations/: ordered migration scripts, one schema operation per file (a table with its
  indexes, a table rebuild, or one trigger group).
- src/connection.rs: SqliteStorage, WAL/pragma setup, and the blocking execution boundary.
- src/filter.rs: compiles the backend-independent filter AST into SQLite SQL.
- src/users.rs, src/scopes.rs, src/views.rs, src/search_sync.rs: focused capability implementations.
- src/smtp_server/: async capability adapter, listener settings, and hashed access credentials.
- src/messages/: async capability adapter, atomic writes, full/metadata snapshot reads, bounded
  summaries and revision hydration, index projections, and normalized child records.
- src/imap/: async capability adapter and cohesive account, mailbox, membership, and flag operations.
- src/records.rs, src/error.rs: shared identifier decoding and backend-neutral error translation.
- src/schema.rs: validates the physical schema against the checked-in scripts.
- src/migrations/: immutable versioned script catalog and the atomic migration runner.

The capability adapters own scheduling onto the shared blocking boundary; their private
operation modules own SQL and transaction boundaries. Helpers remain private to their subsystem
unless another capability actually needs them. The crate root retains its existing public exports.

## Migrations

Schema version 1 is a flattened baseline (scripts 0001-0016): canonical users with a global role,
endpoint memberships, role-less scope memberships, views, mail, and the search outbox. Version 2
(scripts 0017-0019) introduces the single global SMTP server: it creates the `smtp_server`
singleton seeded with `127.0.0.1:1025`, rebuilds `endpoints` without listener columns, and creates
`smtp_accesses` holding only Argon2id password hashes. `migrations/variants/` holds the version 1
layout written by pre-release builds with per-endpoint listeners; such databases are recognized and
inherit their primary listener address as the global server address. Version 3 (scripts 0020-0025)
removes endpoints: users gain the `viewer` role and a `mail_access` column derived from their former
endpoint memberships without widening visibility, scopes, views, SMTP accesses, and mail lose their
endpoint columns (mail keeps identifiers, sequences, revisions, and the sequence high-water mark), and
`endpoint_memberships` and `endpoints` are dropped. Version 4 (script 0026) adds per-access
authentication rules (`requires_encryption`, `plain_mechanism_allowed`, `login_mechanism_allowed`),
defaulting existing accesses to no TLS requirement and both mechanisms. Version 5 (scripts 0027-0035)
adds the IMAP tables: accounts (hashes only), linked Views, local folders, the UIDVALIDITY allocator,
mailbox identities, UID-keyed membership referencing `mail(sequence)` with cascading deletes,
expunge exclusions, flags, and unsubscribed names. The `src/imap/` subsystem reconciles membership by compiling
the supplied definition with the shared filter compiler and assigning UIDs with `ROW_NUMBER()` in one
transaction.

A fresh database applies every batch. A version 1, 2, 3, or 4 database is validated against the exact
schema of its version (for version 1, canonical or the listener variant) and history, then upgraded in the same immediate transaction with foreign-key
enforcement suspended and verified by `foreign_key_check` before commit; existing users,
scope assignments, scopes, views, SMTP accesses, and mail are preserved. Newer, unknown, or structurally invalid
databases are rejected. Creation, upgrade, and the version guard are exercised by the schema
tests.
