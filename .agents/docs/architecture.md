# Architecture

Sandpost captures development mail through SMTP, stores authoritative state in SQLite,
and exposes authenticated HTTP and IMAP. Browser and Electron share Vue; Electron
embeds Rust through a Node-API addon. No external database, queue, or cache is required.

## Ownership and boundaries

- **SQLite is authoritative:** users/roles/mail access, scopes/assignments/Views,
  SMTP server/accesses, IMAP accounts/mailboxes/UIDs/flags/placements, mail facts/raw
  MIME, user deletions, and search outbox. Credentials store hashes only. All mail
  belongs to one pool, without SMTP partitions.
- **Tantivy is disposable:** derived search candidates, recoverable from SQLite;
  never authoritative for access, and stores no raw MIME or attachment binaries.
- **SMTP access:** global administrator-managed submission credential for an
  external application; unrelated to users, scopes, or mail visibility.
- **IMAP account:** user-owned credential bounded by current owner access. Mirror
  shares primary organization, flags, copies and deletion; Independent retains its own
  organization and history. COPY/MOVE share physical content. Mode switching preserves
  dormant Independent state and revokes old sessions. EXPUNGE removes an occurrence;
  a final normal occurrence enters the active view's Trash. See [imap.md](imap.md).
- **Roles** grant authority; **mail access** (`All`/`Scoped`) grants visibility.
  Scope assignments are role-less inherited query restrictions. Personal/shared
  **Views** organize/filter mail, never grant access.

| Crate | Responsibility |
| --- | --- |
| `sandpost-core` | Domain IDs/mail facts, users/roles/access, scopes/Views, SMTP accesses, IMAP accounts/mailbox sources/flags. |
| `sandpost-query` | Typed DSL, canonicalization, validation, exact evaluation. |
| `sandpost-storage` | Backend-independent capability traits, domain/DTO types and semantic errors; no SQL/driver. |
| `sandpost-storage-sqlite` | SQLx backend, schema/migrations, filter-to-SQL compiler; all storage capabilities. |
| `sandpost-search` | Tantivy schema, safe candidates, canonical verification, index lifecycle. |
| `sandpost-application` | Authentication/authorization, management, mail access/events, index coordination; `SmtpAuthenticator`, `ImapService`, `MailListenerControl`. |
| `sandpost-mime` | I/O-free MIME facts/captured bytes; alternative/related selection, boundary recovery, `cid:`/`mid:` resolution, shared attachment transfer decoding for size/hash/download without charset conversion. |
| `sandpost-smtp` | Bounded `SmtpServer`: selected RFC 5321 behavior, SIZE/8BITMIME, AUTH PLAIN/LOGIN, STARTTLS/implicit TLS (rustls), enhanced status/trace fields; MIME parsing and `SessionHandler` delivery. |
| `sandpost-imap` | `ImapServer`: IMAP4rev1 framing/sessions/ENVELOPE/BODYSTRUCTURE/sections/SEARCH; IDLE, UIDPLUS, MOVE, UNSELECT, NAMESPACE, ID, CHILDREN, LITERAL-, SASL-IR, STARTTLS/implicit TLS. `ImapBackend` supplies accounts/mailboxes/UIDs/flags/access; no storage/search dependency. |
| `sandpost-web-api` | HTTP/session middleware/SSE/optional SPA hosting. |
| `sandpost` | CLI/config/bootstrap/listeners, SMTP delivery and IMAP backend adapters, embedded runtime. |
| `sandpost-node` | Electron Node-API boundary. |

Services/application and SQLite depend on the storage contract, never the reverse.
Bootstrap selects one `Arc<dyn Storage>`; SQLite is the sole implemented backend.
MIME/IMAP RFC scope/evidence live in their crates' `RFC-COVERAGE.md`; do not claim
universal RFC certification. Protocol/configuration details: [project.md](project.md)
and [imap.md](imap.md).

## Ingestion, search, and recovery

SMTP checks submitted size before prepending `Return-Path`/`Received`. One SQL
transaction stores mail/child facts and one durable outbox operation; SMTP accepts
after commit. Transport retries can create separate messages.

The worker reads pages of 64, coalesces up to 1024 operations for 64 distinct IDs
into one Tantivy commit, replaces each ID once from current SQL, then acknowledges
only that contiguous prefix. Polling is 10 ms; debounce is 50 ms of quiet, capped
at five seconds under continuous traffic. Remaining batches/shutdown drain without
another debounce. One indexing thread uses a 15 MB heap. Search is eventual.

Exact terms/Unicode trigrams yield safe candidates; approximate plans undergo
canonical fact evaluation. Boolean constants, supported numeric/attachment-presence
predicates and their AND/OR compositions use equivalent index plans without stored
body reads. Identity/revision are fast columns; canonical facts have no separately
duplicated attachment metadata or stored numeric copies. Raw HTML keeps DSL
substring/case/collection semantics, without relevance-token syntax. Short strings/
broad negations may scan widely. SQL hydration checks revision, current filters and
user deletions, refilling bounded sequence-keyset pages past rejected/missing rows.
Direct reads authorize facts/payload from one SQL snapshot. See
[query-language.md](query-language.md) and [scope-engine.md](scope-engine.md).

`CURRENT` names the durable generation. Startup validates schema/checksums and
replays pending work or rebuilds missing/corrupt/behind state. A process-exclusive
installation writer lock prevents competing acknowledgements. Rebuild streams SQL
during ingestion, replays through a cutoff, atomically publishes, then acknowledges;
later work remains pending and in-flight queries retain retired readers.
Only the installation writer acknowledges/prunes the durable outbox. Temporary
coordinators fence rebuilds against concurrent pruning, retrying or explicitly
failing under sustained acknowledgement churn.

Refresh events/SSE `ready` wait for a captured committed watermark via a watch
channel, then recheck visibility/session validity. SMTP waits only for SQL; direct
reads remain immediate. Index-worker failure stops supervised listeners. Shutdown
releases installation/Tantivy writer locks on successful completion even with surviving readers, allowing
native restart; closed writers reject mutations and wake pending waits.
During shutdown, supervision observes transport and indexing task completions
concurrently under one six-second drain deadline. A stalled HTTP event stream
cannot hide a sibling failure that finishes before the deadline.
HTTP shutdown cancels pending handlers and ends active response streams, including
SSE, so accepted connections do not retain application work after listeners stop.
Writer cleanup shares that absolute deadline and runs outside the async executor.
If independently retained work delays cleanup, shutdown reports a timeout; the
cleanup thread continues and retains installation ownership until the writer can
close safely. Synchronous work cannot be forcibly interrupted.
CLI `sandpost search status` reads SQL without the lock; offline `sandpost search
rebuild` requires it and fails during live runtime. Administrator HTTP rebuild
uses the running coordinator's writer gate. Full lifecycle: [storage.md](storage.md).

## Authorization and persistence

Owner bypasses mail access/scopes/personal filters, but user deletions still apply.
Admin manages non-owner users/scopes/SMTP/shared Views/search; Member may delete
visible mail, Viewer cannot. Every role may keep personal Views. `All` needs no
assignments; `Scoped` unions assigned scope/ancestor conjunctions and grants nothing
without assignments. Personal/View/ad hoc filters restrict results. Lists/search/
detail/raw/attachments/events share one composition; neither index documents nor
caller scope IDs authorize reads. Definitions and assignments are reread per
request; in-flight requests may finish using facts read earlier. Hash-bound sessions
invalidate old/racing logins after password changes. Administrators cannot grant,
reset credentials or delete accounts beyond their own authority; owner-only actions
require Owner. Full composition/admin contracts: [scope-engine.md](scope-engine.md).

Web deletion enters primary Trash; permanent deletion inserts a primary tombstone.
Independent references and other primary users retain physical mail. Reference-safe
collection follows final release, with cascades and durable index-outbox deletion.
Permission checks remain separate from retained organization. No active
message-to-scope ingest materialization exists.

File SQLite uses WAL, four read-only connections and one writer; in-memory storage
shares one connection. Initialization is async. Ordered migration names in
`migrations` must form an exact prefix matching physical schema; missing scripts
apply atomically, fresh databases apply all. Future changes append scripts;
unknown names/gaps/schema mismatches fail. No version batches/`PRAGMA user_version`.
Scripts 0026–0056 add deletions/folders/exposures/placement/COPY membership. Older
pre-deployment formats require recreation; startup never resets them automatically.
See [storage.md](storage.md) and [backend-authoring.md](backend-authoring.md).

## Runtime and clients

The `sandpost` runtime keeps service assembly in `lifecycle/mod.rs`, HTTP request/stream
cancellation in `lifecycle/http/mod.rs`, search scheduling in `lifecycle/indexing/mod.rs`,
and coordinated task draining and cleanup in `lifecycle/supervision/mod.rs`.
Configuration, startup reporting, SMTP delivery/listener control, and the IMAP
adapter/listener each retain their own modules. Modules with child files keep their
implementation in `<module>/mod.rs` and unit tests alongside it in `tests.rs`;
test modules with children use `tests/mod.rs`. Real IMAP scenarios are grouped under
`imap_runtime_tests/` by capture, authorization, mailbox organization, modes,
subscriptions, smart-mailbox rights, and retention, with shared wire helpers.

Environment parsing and runtime startup share typed configuration validation.
Embedding hosts can edit configuration values, but startup rechecks TLS requirements,
network IMAP plaintext opt-in, connection limits, and advertised hosts before opening
storage or binding listeners.

Setup is available iff SQLite has no Owner, regardless of non-owner accounts or
`instance.json`. Atomic owner creation preserves accounts/unique email, creates no
session. Browser `/api/v1/setup/authorize` exchanges a boot key for in-memory
setup authorization (10 minutes, maximum 64/oldest eviction, cleared on restart/
initialization); legacy `bootstrapSecret` accepts either. Trusted Electron injects
the boot key via origin-checked IPC without exposing it to Vue. Separate metadata
is written after owner commit, never controls setup, and missing/malformed metadata
does not block initialized instances. Publication failure leaves setup locked.

Global SMTP edits reconcile through `MailListenerControl` without restart.
`SANDPOST_SMTP_LISTEN` overrides persistence for the process lifetime; overridden
management edits do not rebind. Persisted bind failure reports listener failure
without stopping runtime; override bind failure stops startup. Accept failures
are logged/skipped/retried; stopping acceptance releases the port before draining.
AUTH/TLS `auto` policies are reevaluated per bind: AUTH optional on loopback,
required elsewhere; with a certificate, TLS requires STARTTLS off loopback.
`SmtpAuthenticator` checks Argon2id, mechanisms/TLS, last use, enabled state and
unchanged credentials before every delivery. Read/write timeout is 300 s.
Envelope local-part spelling/case survives, domains fold; MIME address normalization
is separate. Blocking MIME work retains its connection lease until completion;
cancellation does not guarantee exactly-once external effects. No outbound mail;
without TLS, credentials are clear text. Configuration: [project.md](project.md).

One IMAP listener serves personal accounts. `ImapService` authenticates/throttles,
revalidates each command and reconciles SQL membership against owner access AND
mailbox definition. Committed mail/visibility/account changes wake IDLE without
polling. TLS/mailbox/service contracts: [imap.md](imap.md).

Vue shares navigation/search/admin behavior. Browser uses HTTP; Electron uses
preload/validated IPC/contextBridge/addon. Preserve sandboxing, renderer isolation,
trusted origins/restricted navigation and captured-HTML isolation. Search shortcuts
become canonical DSL before requests.

Desktop HTML uses an isolated native `WebContentsView` below Vue's toolbar; optional
DevTools occupy a second view attached solely to that email target in the same
window. Email has no Node/preload, a private nonpersistent session, deny-by-default
requests and enforced document policy. Trusted preview IPC supplies visible
rectangles; main converts zoom/bounds. Native views hide for overlays/dragging/
clipping, with iframe browser/clipped fallback. Email target survives message
changes; closing inspector destroys its frontend, viewer/host teardown releases
both. Client/inspector verification: [project.md](project.md).
