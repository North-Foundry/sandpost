# Architecture

Sandpost captures development mail through SMTP, stores authoritative state in SQLite,
and exposes authenticated HTTP and IMAP. Browser and Electron share Vue; Electron
embeds Rust through a Node-API addon. No external database, queue, or cache is required.

## Ownership and boundaries

- **SQLite is authoritative:** users/roles/mail access, scopes/assignments/Views,
  SMTP accesses, IMAP accounts/mailboxes/UIDs/flags/placements, mail facts/raw
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
| `sandpost-web-api` | HTTP/session middleware/SSE/SPA hosting, embedded at build time or served from a directory. |
| `sandpost` | CLI/config/bootstrap/listeners, SMTP delivery and IMAP backend adapters, embedded runtime. |
| `sandpost-node` | Electron Node-API boundary. |
| `sandpost-test-runner` | `smoke-test` verifies real-binary SMTP/HTTP/SSE/IMAP, SQLite/search, startup and restart with independent clients; `mail-burst full`/`tiny` generate finite or repeating SMTP traffic with fixed/random counts and pauses. Both share persistent SMTP sessions. |

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
unknown names/gaps/schema mismatches fail. Each execution records one positive batch for all
pending scripts; reopening without pending work creates none. Offline CLI status reports batches,
and rollback atomically reverses the latest batch without first applying pending scripts.
There is no separate schema version or `PRAGMA user_version`. Validated name-only histories upgrade
to batch 1; older incompatible pre-deployment formats still fail without resetting data.
The 46-script pre-release baseline creates current table definitions directly, with separate
trigger groups and no historical ALTERs or data backfills. Migration 0047 appends durable
remembered-session storage. Earlier development sequences require
explicit database recreation; startup does not reset them.
See [storage.md](storage.md) and [backend-authoring.md](backend-authoring.md).

## Runtime and clients

The `sandpost` runtime keeps service assembly in `lifecycle/mod.rs`, HTTP request/stream
cancellation in `lifecycle/http/mod.rs`, search scheduling in `lifecycle/indexing/mod.rs`,
and coordinated task draining and cleanup in `lifecycle/supervision/mod.rs`.
Configuration, startup reporting, frontend proxying (`frontend_proxy`), offline
maintenance commands (`maintenance/`),
shared client connection hosts (`network/`), SMTP delivery/listener
(`smtp_delivery`, `smtp_listener/`), and the IMAP adapter/listener (`imap_backend/`,
`imap_listener/`) each retain their own modules; `main.rs` only installs tracing and
signals and dispatches commands. SMTP and IMAP certificates load before storage opens. Modules with child files keep their
implementation in `<module>/mod.rs` and unit tests alongside it in `tests.rs`;
test modules with children use `tests/mod.rs`. Real IMAP scenarios are grouped under
`imap_runtime_tests/` by capture, authorization, mailbox organization, modes,
subscriptions, smart-mailbox rights, and retention, with shared wire helpers.

HTTP serves the frontend embedded at compile time when present, otherwise only the API.
Optional `[frontend] proxy_url = "http://frontend:8080"` TOML configuration or
`--frontend-proxy-url http://frontend:8080` enables a frontend proxy in debug and
release builds; the CLI option overrides TOML. The URL is an HTTP or HTTPS origin
using a DNS name or IP address, with default ports 80 and 443 respectively, root
path only, and no credentials, query, or fragment. HTTPS validates the certificate
and hostname against the system trust store.
When configured, API routes remain on Rust and other HTTP and WebSocket traffic is
proxied upstream; an unavailable upstream returns 502. Without a proxy setting,
embedded assets are served when available and otherwise HTTP serves only the API.
Startup reports the configured target as `Frontend proxy`. The root
`sandpost-cli:dev` command uses Turbo to run the ordinary `dev` task concurrently
for Sandpost and its dependency graph, without depending on persistent tasks.
The shared `@sandpost/vite-plugin-hot-file` workspace (`packages/vite-plugin-hot-file`)
exports a TypeScript Vite plugin that finds the workspace root and atomically publishes its
actual origin as plain text to `.vite-hot-file` there. The frontend configuration
uses it for browser, mock and current desktop starts; other Vite configurations
can import the same package. The latest publication wins; normal shutdown removes
the file only if it still contains that server's origin. The crate-local `dev` command runs Cargo with
`--frontend-proxy-file ../../.vite-hot-file`; Rust reads the file
directly and waits up to 30 seconds for publication. The flag, field and reader
are compiled only with debug assertions; release rejects the flag.
`--frontend-proxy-url` is accepted in all profiles and overrides the file in
either argument order without reading or awaiting it. In debug, file contents
use the same origin validation and TOML override as the explicit URL. Proxy-file
discovery is never implicit. Ctrl+C stops both Turbo tasks; a failed task cancels
its sibling. Development HMR uses the backend URL without parsing process logs. The runtime has no external frontend-directory setting.

Environment and configuration-file parsing share typed configuration
validation across CLI and embedded runtime; `SANDPOST_CONFIG` selects the file and an explicitly named file must exist. `SANDPOST_DATA_DIRECTORY` overrides
`storage.data_directory`; an absent or blank file value selects the platform data
default. Relative file data paths resolve from the configuration file's directory.
`SANDPOST_DATABASE_PATH` overrides `storage.database_path`; the selected database
path resolves relative paths against the effective data directory,
preserves absolute paths, and defaults to `sandpost.sqlite3` inside that directory. The desktop
runtime takes no data-directory argument, while Electron keeps its separate userData
directory for browser preferences. Embedding hosts can edit configuration values,
but startup rechecks TLS requirements, network IMAP plaintext opt-in, connection
limits, and advertised hosts before opening storage or binding listeners.

HTTP, SMTP, and IMAP bind addresses and ports follow environment, TOML, then default
precedence. SMTP listener settings are configuration-only and read-only through
management APIs and the interface; changing them requires a restart. IMAP can be
disabled without changing its stored accounts or mail. See [project.md](project.md)
for the complete TOML example and environment variable reference.

Setup is available iff SQLite has no Owner, regardless of non-owner accounts or
`instance.json`. Atomic owner creation preserves accounts/unique email, creates no
session. Browser `/api/v1/setup/authorize` exchanges a boot key for in-memory
setup authorization (10 minutes, maximum 64/oldest eviction, cleared on restart/
initialization); legacy `bootstrapSecret` accepts either. Trusted Electron injects
the boot key via origin-checked IPC without exposing it to Vue. Separate metadata
is written after owner commit, never controls setup, and missing/malformed metadata
does not block initialized instances. Publication failure leaves setup locked.

`MailListenerControl` exposes read-only SMTP listener status from the runtime.
SMTP always binds the resolved configuration address; bind failure stops startup. Accept failures
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
