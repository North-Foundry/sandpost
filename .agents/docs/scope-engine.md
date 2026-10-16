# Scope authorization and query engine

Scopes are instance-wide hierarchical query definitions over one pool of captured mail. Membership only determines which mail reaches a user; it grants no administrative authority. A saved **View** organizes mail and never grants access. Global roles (Owner, Admin, Member, Viewer) govern administration.

## Authorization composition

Each user has one mail-access level. `All` grants all captured mail, subject to personal restrictions, without scope membership. `Scoped` grants the union of assigned scopes, with every ancestor filter inherited:

```text
mail access all:       true
mail access scoped:    effective(scope A) OR effective(scope B) ...
effective assignment:  scope.filter AND parent.filter AND ... AND root.filter
user result:           authorization AND optional personal filter
scoped, no assignments: false
```

`Scoped` with no assigned scopes grants no mail; it never falls back to `All`. Memberships can cover multiple branches, but descendants cannot widen ancestors. Personal filters only restrict. Owners bypass mail access, scopes, and personal filters; administrative actions still require the appropriate global role.

Personal Views are visible to their owner; shared Views have no owner and remain subject to each reader's authorization. Selecting a View adds its filter but creates no membership or assignment.

The application authorization module is the single composition boundary. Search and lists pass its structured plan to the search layer. Direct SQL reads, including identifier-check metadata, apply the same plan to authoritative current SQL facts and payload in one snapshot. This covers detail, raw message, attachment, and event delivery; indexed documents and UIDs are never final authority checks.

User deletion is a separate visibility condition. Lists, search, direct/raw/attachment reads, and event delivery exclude that user's tombstones. Independent accounts retain their own references; physical mail and the global index remain until every primary user and Independent view releases the message. Single deletion requires current authorization; bulk deletion affects only currently visible mail and counts only new tombstones. Repeating a single deletion returns not found.

## IMAP and placement

An IMAP mailbox's membership is its owner's current authorization (mail access plus personal filter; no restriction for owners) AND the mailbox's own filter: linked View, local-folder filter, or INBOX aggregate. Storage reconciles against authoritative SQL facts, never the search index. Definitions are canonicalized and fingerprinted; changes to scopes, assignments, personal filter, role, mail access, or Views trigger synchronization that removes no-longer-authorized members. After such changes commit, `ApplicationChange::Visibility` notifies idling clients so they receive resulting EXPUNGE responses. See [imap.md](imap.md).

Placement is independent of authorization. Mirror shares primary Web placement and copies; Independent uses only its own retained history and placement. Recalculation preserves private organization but never grants access. Web Inbox omits filed primary occurrences, while smart results/global search ignore normal placement. Normal-folder deletion moves otherwise-unfiled occurrences to Trash without clearing tombstones. See [imap.md](imap.md).

## Administration and preview

Authenticated APIs and application management support scopes, assignments, Views, and users. Scope and assignment management requires Owner or Admin. Scope definitions must form an acyclic tree within configured maximum depth, and every filter compiles before saving. An edit affects subsequent requests immediately through current persisted definitions; there is no historical mail reclassification job.

The runtime defaults `sandpost.max_scope_depth` to `2`: roots have depth zero,
children one, and grandchildren two. `SANDPOST_MAX_SCOPE_DEPTH` overrides the TOML
value. Startup validates existing scopes against the resolved limit; installations
with deeper trees must configure a larger value. This validation does not modify
or remove persisted scopes.

`validate_query` (`POST /api/v1/query/validate`) reports the first compilation error and its UTF-8 byte position without saving or executing. `preview_scope` (`POST /api/v1/scopes/preview`) requires a global administrator and samples newest mail matching the unsaved filter plus all parent-ancestor filters, composed with the caller's own authorization. It cannot show mail the administrator cannot read. Deleting a scope with children returns an in-use conflict.

Membership rows are role-less and only contribute to a `Scoped` user's visibility union. Assignments are retained for `All` users but have no effect until mail access becomes `Scoped`; assigning an unknown scope returns not found. There is no message-to-scope materialization pipeline.

## Search candidates and exactness

Tantivy stores sequence, identifier, exact/trigram terms, and serialized facts for canonical verification. Candidate plans must be safe supersets. The search layer evaluates the canonical query against indexed facts and applies shared authorization; indexing may lag SQL, and a candidate hit alone never authorizes or satisfies a search. Exact terms handle exact predicates; trigrams support sufficiently long substrings. Short substrings and broad negative predicates may scan broadly. Never complement an incomplete index to approximate negation.

Results sort by descending SQL message sequence and use exclusive `before` sequence keysets. Search reads bounded candidate batches and hydrates identifiers/revisions from SQL, dropping stale revisions and user tombstones. It continues bounded reads after exclusions so pages are full where more visible results exist and no later result is skipped. Advance by indexed sequence even if physical rows have vanished. The index stays user-independent. Summary rows and child mailbox metadata load from one SQL snapshot. Avoid offset pagination and unbounded reads.

Outbox processing, durable generations, rebuilds, refresh watermarks, and revision synchronization are documented in [storage.md](storage.md). Direct reads check the exact facts and payload in one SQL snapshot. Definition edits need no message revision or reindex; permission revocation affects later requests, while an in-flight request may finish with facts it already read.
