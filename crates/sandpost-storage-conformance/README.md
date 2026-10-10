# sandpost-storage-conformance

Reusable behavior checks for any implementation of the SandPost storage contract. This crate
contains no database driver and depends only on the shared domain, query, and storage contracts.

Call `run_all(&dyn Storage)` against an empty, isolated backend. It creates its own fixtures,
runs checks in a fixed order, and returns the first named `ConformanceFailure`. Individual checks
remain available through the crate root; checks that need existing users create their own fixtures,
while bootstrap explicitly needs an empty backend.

## Source organization

- `lib.rs`: public check exports, failure reporting, health, and suite orchestration.
- `users/`: account management and global-role invariants.
- `smtp_server/`: listener configuration, hashed credentials, and authentication rules.
- `scopes.rs`, `views.rs`: hierarchical scopes, memberships, and saved views.
- `messages/`: record lifecycle and pagination, equivalence with the canonical query evaluator,
  and index projections with durable revisions.
- `search.rs`: durable outbox ordering, batching, and acknowledgement.
- `imap/`: account configuration, mailbox identities and membership, flags and non-destructive expunge.
- `support.rs`: shared assertions and isolated domain fixtures, with IMAP fixtures local to `imap/`.

A backend's integration test owns connection creation and calls this suite. Driver-specific
migration, locking, and physical-schema checks belong in the backend's own tests.
