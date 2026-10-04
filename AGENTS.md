# Project instructions

Repository guidance for coding agents is collected in `.agents/docs/`.
Consult the documents that apply to a task before editing the project:

- `.agents/docs/project.md` — application behavior, configuration, API,
  development workflow, verification, and planned work.
- `.agents/docs/architecture.md` — system components, crate responsibilities,
  application flows, deployment, and current limitations.
- `.agents/docs/storage.md` — persistence contracts, SQLite schema and startup,
  transactions, visibility materialization, and concurrency limits.
- `.agents/docs/query-language.md` — query syntax, types, collection semantics,
  canonicalization, fingerprints, and safety limits.
- `.agents/docs/scope-engine.md` — inherited visibility, candidate selection,
  shared evaluation, policy versions, and materialization updates.
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
