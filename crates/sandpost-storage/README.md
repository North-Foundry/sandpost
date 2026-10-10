# sandpost-storage

Backend-independent persistence contract for SandPost.

This crate defines the storage capabilities the application depends on and the
backend-independent domain types they exchange. It contains no database driver and no SQL.
A concrete backend (currently sandpost-storage-sqlite) implements the capability traits and is
selected once at the composition boundary.

## Capabilities

- UserStorage: canonical users, credentials, global roles, mail access, bootstrap.
- SmtpServerStorage: the global SMTP server address and hashed SMTP access credentials.
- ImapStorage: user-owned IMAP accounts (hashed credentials), their linked Views and local folders,
  mailbox identities with persistent UIDs, materialized membership, and IMAP flags.
- ScopeStorage: hierarchical mail scopes and role-less memberships.
- ViewStorage: personal and shared saved views.
- MessageStorage: atomic ingestion, retrieval, ordering, deletion, and index projection.
- SearchSynchronizationStorage: the durable search outbox protocol.
- StorageHealth: liveness probing.

Storage is the composition of all of them with a blanket implementation:

```text
Application
    |
    v
Arc<dyn Storage>
    |
    v
UserStorage + SmtpServerStorage + ImapStorage + ScopeStorage
  + ViewStorage + MessageStorage + SearchSynchronizationStorage + StorageHealth
```

## Backend neutrality

- No rusqlite, SQL text, connection, transaction, or backend error type appears here.
- Errors are semantic StorageError values; a backend maps its own errors into them.
- Filters are the shared canonical query AST, never a SQL fragment.
- Atomicity is a property of a storage operation, not of an exposed transaction object.

See .agents/docs/storage.md and .agents/docs/backend-authoring.md for the logical model and the
third-party backend guide.

## Source organization

`capabilities/` groups trait contracts by domain and keeps their blanket composition in
`storage.rs`. `types/` groups non-IMAP requests and results by domain. `imap.rs` defines the
IMAP contract, with its account, folder, mailbox, member, flag, and copy types in `imap/types/`.
The small facades and crate root use explicit re-exports to keep the public import paths stable.
