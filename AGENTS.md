# Project instructions

Repository guidance for coding agents is collected in `.agents/docs/`.
Consult the documents that apply to a task before editing the project:

- `.agents/docs/project.md` — current application behavior, global/endpoint roles and mail-access authorization, search operations,
  configuration, API, frontend/Electron workflow, verification, and limitations.
- `.agents/docs/architecture.md` — system components, crate responsibilities,
  application flows, deployment, and current limitations.
- `.agents/docs/storage.md` — the backend-independent persistence contract and the SQLite backend schema/migrations,
  async execution, transactions, search outbox and derived-index lifecycle, and concurrency limits.
- `.agents/docs/backend-authoring.md` — backend authoring: implementing a new storage backend against the contract.
- `.agents/docs/query-language.md` — query syntax, types, collection semantics, canonical verification, fingerprints,
  search candidate limits, and safety bounds.
- `.agents/docs/scope-engine.md` — scope membership and endpoint mail-access composition, inherited filters, search candidates,
  exact verification, and durable index synchronization.
- `.agents/docs/code-style.md` — complete Rust and SQL names and documentation
  conventions for functions, helpers, and tests.
- `.agents/docs/commits.md` — commit format and workflow.

Maintain this index when a task changes `.agents/docs/`: every path and
description must match the documents present. Tasks outside that directory do
not require an index review. Index only documents inside `.agents/`.

Use `.agents/docs/` as the source of project conventions and contracts. Apply
its relevant guidance rather than introducing competing conventions. For
changes to query semantics, scope visibility, or persistence, consult the
corresponding document and keep the implementation, tests, and documentation
consistent. Distinguish implemented behavior from planned features.

Keep this root `AGENTS.md` as the single project instruction entry point.
