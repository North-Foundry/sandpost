# Project instructions

This is the single project instruction entry point. Use `.agents/docs/` as the
source of conventions/contracts; read applicable documents before editing:

- `.agents/docs/project.md` — branding, application behavior, roles/mail access,
  SMTP/IMAP, configuration/API, frontend/Electron workflow, verification and limits.
- `.agents/docs/architecture.md` — components, crate boundaries, flows, deployment and limits.
- `.agents/docs/storage.md` — persistence contract, SQLite schema/migrations and
  migration-file conventions, async execution, transactions, user deletion,
  search outbox/index lifecycle and concurrency.
- `.agents/docs/backend-authoring.md` — implementing a storage backend against the contract.
- `.agents/docs/query-language.md` — syntax/types/collections, canonical verification,
  fingerprints, candidate limits and safety bounds.
- `.agents/docs/scope-engine.md` — memberships/mail-access composition, inherited
  filters, candidates, exact verification and durable synchronization.
- `.agents/docs/imap.md` — accounts/credentials, virtual mailbox and Web projections,
  placement/Trash, membership/UIDs/flags/user deletion, IDLE, configuration and limits.
- `.agents/docs/code-style.md` — complete Rust/SQL naming and documentation rules
  for functions, helpers and tests.
- `.agents/docs/commits.md` — commit format and workflow.

When changing `.agents/docs/`, check that this index's paths/descriptions match
all documents present; index only documents inside `.agents/`. Other changes do
not require an index review.

Apply these conventions rather than introducing competing ones. Query, scope
visibility and persistence changes must keep implementation, tests and the
corresponding documentation consistent. Distinguish implemented from planned behavior.
