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

- migrations/: ordered baseline scripts, one schema operation per file (a table with its indexes,
  or one trigger group).
- connection.rs: SqliteStorage, WAL/pragma setup, and the blocking execution boundary.
- filter.rs: compiles the backend-independent filter AST into SQLite SQL.
- users.rs, endpoints.rs, scopes.rs, views.rs, messages.rs, search_sync.rs: capability impls.
- schema.rs, migrations.rs: schema validation and baseline initialization.

## Migrations

Schema version 1 is a single flattened baseline applied from ordered per-operation scripts:
SandPost has not been deployed, so a fresh database is created directly at the current schema
(canonical users with a global role, endpoint memberships, role-less scope memberships, views,
mail, and the search outbox) and any other stored version is rejected rather than migrated.
Baseline creation and the version guard are exercised by the schema tests.
