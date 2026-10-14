# Sandpost

Sandpost is a self-hosted development email catcher with a shared Vue mail
interface, Rust application/runtime, SMTP ingestion, authenticated HTTP API,
and an optional Electron desktop shell with a Rust Node-API addon. It includes
canonical user accounts with global roles (Owner, Admin, Member, Viewer) and a per-user
mail access, one global SMTP server with centrally administered SMTP access credentials,
hierarchical scopes, personal/shared saved Views, and one IMAP service through which every user can
connect mail clients with personal IMAP accounts. Mail access and scope assignments authorize
messages; Views organize mail and do not grant access. All captured mail forms one pool. Primary organization and deletion belong to each Sandpost user and are shared by Mirror
IMAP accounts. Independent accounts retain their own folders, flags, Trash and history,
bounded by current owner permissions. Physical message content remains shared until all
retaining views release it.

## Product branding

The product name is **Sandpost**, one word with only the initial **S** capitalized. Use it in user-facing text, documentation, window titles, and distribution names. Technical names retain their required spelling: `sandpost` crates/packages, `sandpost://` URLs, `SANDPOST_*` variables, and existing paths such as `SandPost.app`.

## Build and run

Requirements: Rust 1.99+, Cargo, a C compiler (bundled SQLite), Node.js 22.12+, and Yarn 4. Install JavaScript dependencies at the repository root; Rust crates share the root workspace and lockfile.

```sh
yarn install
cargo build --workspace --locked
cargo run -p sandpost --locked
```

Defaults are HTTP `127.0.0.1:8025`, SMTP `127.0.0.1:1025`, and IMAP `127.0.0.1:1143`. Without `SANDPOST_FRONTEND_DIR`, HTTP serves only the API. For a web UI, run `yarn frontend:build` and set `SANDPOST_FRONTEND_DIR=apps/frontend/dist`; startup requires its `index.html`. The server serves assets and uses that file for non-API SPA paths; unknown `/api` paths remain 404. Node is a build dependency only. Ship `dist` beside the Rust binary when enabling the UI.

Root scripts: `yarn frontend:dev` runs Vite with the explicit mock client; `yarn frontend:dev:http` proxies HTTP to the local Rust server; `yarn frontend:build` type-checks and builds; `yarn frontend:test` runs frontend tests. `yarn desktop:dev` builds the native addon and Electron main/preload, runs Vite on 1420, and launches Electron; browser Vite uses 5173. `yarn native:build` builds/copies the napi-rs 3 addon to `apps/desktop/native/<platform>-<architecture>/sandpost.node`; `yarn native:test`, `yarn desktop:check`, `yarn desktop:test`, `yarn desktop:smoke`, and `yarn desktop:smoke --dev` exercise the native, IPC, and Electron paths. `yarn desktop:build:dir` creates an unsigned unpacked app; `yarn desktop:build` packages the current host. Electron ships Vue assets and native artifacts outside ASAR; packaged users need no toolchain. Targets are macOS ARM64/x64, Linux x64, and Windows x64; only CI/local-built targets count as validated. Signing and notarization are not configured.

The mock has 24 synthetic messages and supports inbox creation, search, deletion, decoded attachments, raw MIME, keyset pages, and ready events. Its mutations last only for the client lifetime and do not exercise Rust. Normal Vite/build uses HTTP; Electron's explicit `window.sandpost` preload API takes precedence over mock configuration. Browser bundles contain no Electron imports or native shims.

Send a test message with Python's standard library (the CLI prints the session-protected list endpoint after startup):

```sh
python3 - <<'PYMAIL'
import smtplib
from email.message import EmailMessage
m = EmailMessage()
m['From'] = 'dev@sender.test'
m['To'] = 'developer@recipient.test'
m['Subject'] = 'Hello Sandpost'
m.set_content('Captured locally.')
with smtplib.SMTP('127.0.0.1', 1025) as smtp:
    smtp.send_message(m)
PYMAIL
```

The CLI stores data in `$HOME/.sandpost` by default: SQLite is `sandpost.sqlite3`, derived search state is under `search`, and installation metadata is `instance.json` (metadata does not determine setup state). Facts/attachment metadata use relational rows; original MIME, including attachment bytes, stays in SQLite. SMTP commits before acknowledging mail. SIGINT/SIGTERM stop listeners with bounded draining; embedded runtimes expose shutdown and drain on host exit.

After storage/listener initialization, stdout prints a plain-text connection summary; tracing goes to stderr, colors require a TTY and no `NO_COLOR`. It reports actual API/bind addresses, SMTP host/port and encryption/AUTH state, SQLite path, and IMAP connection/security or disabled status. Wildcard binds map client instructions to the matching loopback family while `Listening` shows the actual bind; port 0 reports the allocated port. A failed persisted SMTP bind appears as status; an SMTP environment override bind failure stops startup. SMTP server address is persisted and can be moved/stopped by an administrator while running; `SANDPOST_SMTP_LISTEN` overrides it for the process lifetime.

When no Owner exists, each startup prints a fresh 256-bit setup key and local `/setup` URL. `GET /api/v1/instance` reports `{ initialized, setupKeyRequired }`; the browser verifies the key at `POST /api/v1/setup/authorize`, then submits Owner fields with the temporary authorization in `bootstrapSecret` to `POST /api/v1/setup`. Authorizations expire in 10 minutes, at most 64 are kept (oldest evicted), and restart or successful setup invalidates them. The legacy field also accepts the boot key. Setup creates no session; login follows. The key is never exposed by HTTP or persisted in frontend storage. Setup remains available whenever storage has no Owner, even if non-Owner accounts or `instance.json` exist; Owner creation is atomic, preserves those accounts, and honors unique email. Metadata is written after Owner creation; a metadata-write failure locks setup for that process, while restart recognizes the committed Owner.

Setup validation/server errors appear inline. If another client initializes the instance, setup exits to login or the authenticated app; refresh rereads status without persisting the key or authorization.

Electron is trusted: native status says no setup key is required, and `NativeRuntime::setup` injects the boot key through origin-checked IPC; the renderer supplies only Owner fields. The mock uses `sandpost-test-secret`, the same authorization flow, and a 12-byte minimum password.

For containers, mount `/data` persistently and set `SANDPOST_DATA_DIR` (replace `IMAGE`):

```sh
docker run --name sandpost --rm \
  -p 127.0.0.1:8025:8025 -p 127.0.0.1:1025:1025 \
  --mount type=volume,source=sandpost-data,target=/data \
  -e SANDPOST_HTTP_LISTEN=0.0.0.0:8025 \
  -e SANDPOST_SMTP_LISTEN=0.0.0.0:1025 \
  -e SANDPOST_IMAP_LISTEN=off \
  -e SANDPOST_DATA_DIR=/data IMAGE
```

Read the key using `docker logs sandpost`. The volume preserves SQLite and `instance.json`. A non-loopback SMTP bind requires AUTH by default; create an SMTP access or explicitly set `SANDPOST_SMTP_AUTH=optional`. The example disables IMAP. Non-loopback IMAP needs a certificate (`SANDPOST_IMAP_TLS_CERTIFICATE`/`_PRIVATE_KEY`, or the SMTP pair) and a published port, e.g. `-p 127.0.0.1:993:993 -e SANDPOST_IMAP_LISTEN=0.0.0.0:993`; without a certificate startup refuses it unless `SANDPOST_IMAP_TLS=off`, which permits clear-text passwords.

`python3 scripts/mail_bursts.py` sends repeating traffic; `--cycles 1` sends one cycle (106 messages). Each cycle sends 100 immediately, waits 30 seconds, then sends 6 at five-second intervals. Defaults are `127.0.0.1:1025`, overridable with `--host`/`--port`; pass `--username` and set `SANDPOST_SMTP_PASSWORD` for AUTH, keeping the password out of command arguments.

### Security and native preview

Electron enables renderer context isolation/sandboxing and disables Node integration. IPC validates the owned window, main frame and trusted origin; preload exposes no filesystem or shell access.

HTTP has no built-in TLS. Without SMTP/IMAP certificates, credentials and HTTP sessions are clear text; keep listeners on loopback or protect them with a trusted boundary/TLS-terminating proxy. SMTP AUTH defaults to optional on loopback and required elsewhere; TLS `auto` requires STARTTLS on non-loopback when a certificate exists. IMAP clear text is loopback-only unless explicitly opted out of TLS. See Configuration and [imap.md](imap.md) for exact protocol and mailbox behavior.

Browser HTML preview uses an empty-sandbox iframe and restrictive content policy. Electron uses a dedicated `WebContentsView` with no preload, Node access, application session, or persistent storage; a private session serves only the current document through `sandpost-email:`. Response CSP includes an empty `sandbox`; scripts, remote/application resources, nested frames, connections, permissions, downloads, popups, and navigation are blocked, while inline CSS and data images work. The same wrapped document is used in both previews; no separate HTML sanitizer is claimed.

The desktop-only Inspect control attaches a fresh DevTools `WebContentsView` to the email target inside the existing window (no extra window and never the app renderer). It starts closed at about 35% width, with pointer/keyboard divider resizing. The email target survives message changes; generation checks discard stale loads. Chromium's bundled settings place the first Elements/Styles sidebar on the right, preserve later window-session settings, and apply the resolved light/dark theme, including system appearance. The inspector stays hidden until settings/theme are ready and its first themed frame is painted; Vue shows a themed animated loader until readiness and a scroll hint only when a ready inspector is out of view. Reduced-motion is honored.

Vue sends viewport rectangles over `sandpost:preview:*` IPC, validating the owned application main frame and payloads; main converts them using renderer zoom and updates native bounds on scroll/resize/layout. Native views hide while dragging, overlays are open, or either region is clipped; the iframe remains the clipped fallback. The inspector needs both regions fully visible and is bounded by the scrollable viewer/minimum window. Failures fall back to browser preview. Both views are destroyed on unmount or host navigation/crash/close. In pinned Electron 44.5.1, the caller-owned DevTools frontend survives `closeDevTools()`/`close()`, so cleanup calls the native `destroy()` binding (missing from TS declarations); smoke coverage checks cleanup/reopen. Recheck Chromium settings and this binding on Electron upgrades.

## Configuration

| Environment variable | Default | Meaning |
| --- | --- | --- |
| `SANDPOST_HTTP_LISTEN` | `127.0.0.1:8025` | HTTP socket address |
| `SANDPOST_SMTP_LISTEN` | unset | Optional override of the global SMTP server's persisted address (seeded as `127.0.0.1:1025`) |
| `SANDPOST_SMTP_AUTH` | `auto` | SMTP AUTH requirement: `auto` (optional on loopback, required elsewhere), `required`, or `optional` (explicit opt-in to unauthenticated mail on any address) |
| `SANDPOST_SMTP_TLS` | `auto` | SMTP TLS requirement: `auto` (with a certificate, required on non-loopback addresses and offered on loopback; without one, no TLS), `required` (needs a certificate), or `off` |
| `SANDPOST_SMTP_TLS_MODE` | `starttls` | `starttls` (clients upgrade with STARTTLS, ports 25/587) or `implicit` (TLS from the first byte, port 465; needs a certificate) |
| `SANDPOST_SMTP_TLS_CERTIFICATE`, `SANDPOST_SMTP_TLS_PRIVATE_KEY` | unset | PEM certificate chain and private key for SMTP TLS; set both or neither. An unreadable or mismatched pair stops startup |
| `SANDPOST_IMAP_LISTEN` | `127.0.0.1:1143` | IMAP listener address, or `off` to disable IMAP |
| `SANDPOST_IMAP_TLS` | `auto` | IMAP TLS requirement: `auto` (with a certificate, TLS; STARTTLS mode requires it on non-loopback addresses; without one, clear text allowed only on loopback), `required` (needs a certificate), or `off` (explicit opt-in to clear-text passwords on any address) |
| `SANDPOST_IMAP_TLS_MODE` | `implicit` | `implicit` (IMAPS, port 993) or `starttls` (port 143) |
| `SANDPOST_IMAP_TLS_CERTIFICATE`, `SANDPOST_IMAP_TLS_PRIVATE_KEY` | the SMTP pair | PEM certificate chain and key for IMAP; set both or neither. An unreadable or mismatched pair stops startup |
| `SANDPOST_IMAP_HOST` | bound address | Host name shown in IMAP connection instructions |
| `SANDPOST_IMAP_MAX_CONNECTIONS` | `64` | Concurrent IMAP sessions (1 to 10000) |
| `SANDPOST_SMTP_ENDPOINT`, `SANDPOST_SMTP_ENDPOINTS` | removed | SMTP endpoints no longer exist; setting either fails startup with a migration hint |
| `SANDPOST_DATA_DIR` | `$HOME/.sandpost` | Local data directory; falls back to `$USERPROFILE/.sandpost`, then `data` when neither home variable is available |
| `SANDPOST_DATABASE_PATH` | `<data directory>/sandpost.sqlite3` | SQLite file |
| `SANDPOST_MAX_SCOPE_DEPTH` | unset, unlimited | Maximum depth; roots are depth zero |
| `SANDPOST_LOG_LEVEL` | `info` | Tracing filter, e.g. `info,sandpost_application=debug` |
| `SANDPOST_FRONTEND_DIR` | unset | Optional built frontend directory; must contain `index.html` |


CLI's new data default does not migrate existing relative `data/`; set `SANDPOST_DATA_DIR=data` or move the complete directory. Desktop uses the platform app-data path only when `SANDPOST_DATA_DIR` is unset; explicit data/database paths and other shared runtime configuration are honored.

Malformed configuration, invalid persisted topology/filter, failed migrations, missing configured frontend index, removed SMTP endpoint variables, search writer-lock conflicts, HTTP/IMAP bind errors, unusable IMAP certificates, non-loopback IMAP without TLS (unless explicitly disabled), and an SMTP override bind error stop startup. An unbindable persisted SMTP address does not stop startup; it is reported on SMTP Access. SQLite is authoritative; Tantivy is recovered/rebuilt as a derived index.

The embedded runtime revalidates configuration before opening storage or binding
listeners, including settings edited after environment parsing. Both entry paths
enforce the same TLS requirements, network IMAP plaintext opt-in, IMAP connection
limits, and advertised-host validation.

Shutdown cancels pending HTTP handlers and active response streams, including SSE.
Transport draining and search-writer cleanup share a six-second host deadline.
Cleanup runs on a separate thread; if retained work or synchronous disposal exceeds
the deadline, shutdown returns a timeout and cleanup continues in the background,
retaining writer ownership until release is safe.

SMTP is a bounded capture server with selected RFC behavior, not universal compliance: SMTP command/reply (RFC 5321), SIZE (1870), 8BITMIME (6152), AUTH framing (4954), PLAIN (4616), LOGIN, STARTTLS (3207), implicit TLS (8314), enhanced status codes (2034), and trace fields (3848). EHLO advertises PIPELINING, SIZE, 8BITMIME, ENHANCEDSTATUSCODES, configured STARTTLS, and permitted AUTH. Unknown commands return 500, unsupported standard commands 502, malformed commands 501, and VRFY 252 without mailbox confirmation.

Limits: 10 MiB/message, 100 recipients/transaction, 32 concurrent sessions (excess gets 421), 3 failed AUTH attempts/session, 300-second network read/write, 60-second handler, 30-second TLS handshake, and 15-minute session. Lines: ordinary commands 512 bytes including CRLF; extended MAIL 1,038; AUTH continuations 12,288; unstuffed DATA 1,000; replies 512. EHLO is required for AUTH/ESMTP parameters and HELO/EHLO before MAIL. Envelope paths are strict ASCII; local-part case is preserved and domain lowercased. Bare Postmaster resolves to `Postmaster@<server_name>` (default `sandpost.localhost`). Duplicate/malformed parameters, AUTH base64, and AUTH xtext fail. SIZE applies before trace fields; capture prepends `Return-Path`/`Received`, removes earlier Return-Path fields, and preserves body/attachments.

Accept errors retry. Shutdown releases the port and bounds async draining, but blocking MIME parsing can retain a session lease past shutdown. Handler cancellation/timeout cannot undo side effects or guarantee exactly-once effects. No outbound delivery; omitted extensions include SMTPUTF8, CHUNKING/BDAT, DSN, EXPN, TURN, ETRN, ATRN, BURL, and client-certificate authentication.

#### SMTP TLS and access credentials

SMTP uses rustls. STARTTLS-required sessions cannot AUTH (`538`) or MAIL (`530`) before upgrade; the client greets again afterward and prior plaintext auth is cleared. Pipelined commands behind STARTTLS close the connection. TLS policy follows the bound address and is reported as none, optional/required STARTTLS, or implicit TLS. Certificates load at startup; rotation needs restart.

SMTP accesses are global credentials, unrelated to Sandpost users, scopes, or visibility; only Owners/Admins manage them. Passwords are 32 secure-random characters from an unambiguous alphabet (~185 bits). Only salted Argon2id PHC hashes are stored (19 MiB, 2 iterations, 1 lane). Plaintext appears once in a `Cache-Control: no-store` create/rotate response; listings/status/server responses and logs contain no password/hash. Rotation immediately invalidates the prior password. AUTH uses dummy-hash verification for unknown usernames and at most four concurrent verifications; disabled credentials fail, use time is recorded at most once/minute, and every delivery rechecks existence/enabled/hash (failure 554). Mail from all accepted sources joins the same pool; source credential is not stored on the message.

Each access requires TLS or not and allows one or both of PLAIN/LOGIN (at least one; new accesses allow both and do not require TLS through API). Rules are checked after password verification to avoid username disclosure. TLS-only clear-text use returns `538 5.7.11`; disallowed mechanism `534 5.7.9`; neither counts as a failed attempt. Tightened rules reject delivery from stale sessions with 554. UI defaults new access to TLS-only when server encryption is available and warns when no certificate can support that choice. SMTP accesses cannot be renamed; create another access.

#### IMAP accounts

Every user can create up to 25 personal IMAP credentials with a unique username,
ephemeral generated password, Argon2id hash, enabled state, and explicit Mirror or
Independent mode. Mirror shares primary Web folder hierarchy, copies, supported flags,
Trash, smart results and permanent removals. Independent owns a snapshot of current
primary history and normal organization, its own flags/Trash and account smart definitions.
Later arrivals enter dormant Independent retention even while Mirror is active. Mode
switches preserve state and revoke connected sessions; clients reconnect into the other
UID namespace. Current owner authorization always bounds both modes.

Normal CREATE/RENAME/DELETE affect the active view. Hierarchy parent renames preserve
UIDs and descendants. Deleting a folder trashes only otherwise-unfiled occurrences;
other normal copies survive. Smart SELECT permits only Seen updates; membership changes
and non-Seen STORE are refused. APPEND/import remains unsupported. Deleted is a local
occurrence marker; EXPUNGE promotes surviving copies, moves a final normal occurrence
to active-view Trash, and releases a final Trash occurrence. Independent deletion never
modifies primary/peer state. Credential deletion removes private state and can collect
content retained by no other view. Full contract: [imap.md](imap.md).

Web normal folders use `GET/POST /api/v1/folders`, `PUT/DELETE /api/v1/folders/<id>` and
`GET /api/v1/folders/<id>/messages`; the body is `{ name }`, while the legacy exposure
field remains accepted. Every Mirror sees them; Independent receives its own initial
snapshot only. Web MOVE uses `POST /api/v1/messages/placement` with `{ identifiers,
folderIdentifier }` (null means Inbox). Web COPY preserves sources through
`POST /api/v1/messages/copy` with the same body. Batches are limited to 256 messages.
Read status and supported flags belong to primary state and synchronize with Mirror.
`POST /api/v1/messages/seen` accepts `{ identifiers, seen }`; `/messages/flags` accepts
`{ identifiers, operation, flags }`, where operation is add/remove/replace and flags are
Seen/Answered/Flagged/Draft/keywords. Deleted remains occurrence-only and is refused
by the primary flags API. Summaries expose those shared flags.

Deleting outside Web Trash moves the underlying message there without confirmation
(`POST /api/v1/messages/trash`, max256, returns `{ trashed }`; Viewer denied). It leaves
ordinary smart/search results. `GET /api/v1/trash/messages` lists primary Trash; MOVE
restores mail. Deleting in Trash uses explicit permanent DELETE after confirmation;
Empty Trash (`DELETE /api/v1/trash`) releases the primary view. Independent references
retain content separately. Electron mirrors the same operations through validated IPC.

Use the account page's host/port, username/password, and displayed security. Default loopback is `127.0.0.1:1143`, no TLS (Apple Mail: uncheck Use TLS/SSL; Thunderbird: Connection security None, Authentication Normal password); otherwise choose SSL/TLS or STARTTLS as configured. Self-signed certificates require manual trust.

## HTTP API and frontend behavior

Public Rust and HTTP representations retain their existing field names.

| Method and path | Result |
| --- | --- |
| `GET /api/v1/health` | Readiness including database and active index probes |
| `GET /api/v1/instance` | Public status `{ initialized, setupKeyRequired }`; never reveals the bootstrap secret |
| `POST /api/v1/setup/authorize` | Verify `{ setupKey }` and return a temporary `{ authorization }`; creates no Owner |
| `POST /api/v1/setup` | Complete first-owner setup using the boot key or temporary authorization in `bootstrapSecret` |
| `POST /api/v1/login` | Verify credentials and issue a session |
| `GET /api/v1/session`, `POST /api/v1/logout` | Inspect/revoke the current session |
| `GET/PUT /api/v1/smtp/server` | Global administrators: read or change the global SMTP server address (`listen_address`, null disables); the response includes the runtime listener status and whether AUTH is required |
| `GET/POST /api/v1/smtp/accesses` | Global administrators: list SMTP accesses (never secrets) or create one from `{ name, username?, requires_encryption?, allowed_mechanisms? }` (mechanisms `plain`/`login`); creation returns `{ access, password, server }` once with 201 |
| `PUT/DELETE /api/v1/smtp/accesses/<id>` | Global administrators: partially update `{ enabled?, requires_encryption?, allowed_mechanisms? }` or delete an access |
| `POST /api/v1/smtp/accesses/<id>/password` | Global administrators: regenerate the password and return it once; the previous one stops working immediately |
| `GET/POST /api/v1/scopes` | Global administrators: list/create or update scope query definitions |
| `POST /api/v1/scopes/preview` | Sample the newest mail an unsaved scope filter would select, with inherited filters and the caller's visibility |
| `POST /api/v1/query/validate` | Report the first DSL compilation error and its UTF-8 byte position without running the query |
| `PUT/DELETE /api/v1/scopes/<id>` | Global administrators: update/delete a scope |
| `GET /api/v1/scopes/<id>/assignments` | Global administrators: list users assigned to a scope |
| `PUT/DELETE /api/v1/scopes/<id>/assignments/<user>` | Global administrators: assign/unassign a user |
| `GET/POST /api/v1/views` | List/create personal saved Views (any user) and shared ones (global administrators) |
| `PUT/DELETE /api/v1/views/<id>` | Update/delete a View |
| `GET/POST /api/v1/users` | Global administrators: list/create users with a global role and `mail_access` (`all` or `scoped`, default `scoped`) |
| `PUT/DELETE /api/v1/users/<id>` | Global administrators: update the name, password, role, or mail access, or delete a user |
| `GET /api/v1/inboxes` | Compatibility inbox surface backed by Views, including virtual Default Inbox |
| `POST /api/v1/inboxes` | Create a compatibility inbox/View |
| `GET /api/v1/messages?limit=50&before=<seq>&view_id=<uuid>&q=<DSL>` | Newest first, keyset pagination, max 100; authorization and caller deletion state enforced. Ordinary/view lists follow unfiled Web placement; q without a view is global search including filed mail |
| `GET /api/v1/search?q=<DSL>&limit=50&before=<seq>&view_id=<uuid>` | Authorized DSL query search; bounded keyset pages exclude the caller's deleted messages |
| `GET /api/v1/search/status` | Administrator search synchronization status |
| `POST /api/v1/search/rebuild` | Administrator derived-index rebuild |
| `GET /api/v1/messages/<uuid>` | Authorized normalized facts and attachment metadata; deleted messages return 404 |
| `GET /api/v1/messages/<uuid>/raw` | Authorized original captured `.eml`; deleted messages return 404 |
| `GET /api/v1/messages/<uuid>/attachments/<index>` | Authorized decoded MIME attachment bytes; deleted messages return 404 |
| `DELETE /api/v1/messages/<uuid>` | Persistently hide a visible message for the caller by any role but Viewer; already-deleted or unauthorized messages return 404 |
| `DELETE /api/v1/messages` | Persistently hide all currently visible messages for the caller; `{ deleted }` counts newly hidden messages |
| `GET /api/v1/events` | Authenticated SSE notifications for authorized mail |
| `POST /api/v1/messages/seen` | Set primary read status with `{ identifiers, seen }`, maximum 256; returns `{ changed }`; shared by Mirror accounts |
| `GET /api/v1/imap/server` | Any user: IMAP connection details `{ enabled, host, port, security, listen_address }` (`security` is `none`, `start_tls_optional`, `start_tls_required`, or `implicit`) |
| `GET/POST /api/v1/imap/accounts` | Any user: list the caller's IMAP accounts with `linked_views`, `folders`, and the exposed `mailboxes`, or create one from `{ name, username?, mode? (mirror by default) }`; creation returns `{ account, password, server }` once with 201 and `no-store` |
| `GET/PUT/DELETE /api/v1/imap/accounts/<id>` | The caller's own account: read, partially update `{ name?, enabled?, mode? }`, or delete (private state is removed; unreferenced mail may be collected); other users' accounts are 404 |
| `POST /api/v1/imap/accounts/<id>/password` | Regenerate the password and return `{ account, password, server }` once; the previous password and running sessions stop working |
| `POST /api/v1/imap/accounts/<id>/restore` | Clear legacy mailbox exclusions; preserves placement and cannot restore permanently released view references; returns `{ restored }` |
| `POST /api/v1/imap/accounts/<id>/folders` | Create an Independent account smart folder from `{ name, filter }`; canonical filter validation applies (400), names are account-local (409), maximum 100 folders. Legacy Mirror owner definitions remain compatibility-only |
| `PUT/DELETE /api/v1/imap/accounts/<id>/folders/<folder>` | Update or delete the Independent account smart definition; Mirror definitions are primary-owned |


Search `q` is always DSL at `/api/v1/search`; `messages.search` adapts literal text to `content contains <DSL string literal>`, and the native helper emits the equivalent canonical Content predicate. Vue normalizes its search box and calls `messages.list({ query })`, mapped to `GET /api/v1/messages?q=` or Electron `messagesList`. Message listing accepts `view_id` (or compatibility `inbox_id`) and DSL `q`. Search is global over physical mail, including filed mail; user tombstones and the user's Web Trash filter only that user's results. CLI maintenance is `sandpost search status` and `sandpost search rebuild`: status reads SQL without the writer lock, offline rebuild needs the installation lock and cannot run under a live runtime, while HTTP rebuild uses the running coordinator gate.

Health, instance status, setup authorization, first-owner setup, and login are public; static SPA routes are public and unknown API paths stay 404. Other API routes require a session; application services enforce roles and message visibility. Browser bearer tokens remain in memory; login also sets an `HttpOnly`, `SameSite=Strict` cookie for SSE. Cookie-authenticated mutations require same-origin `Origin`. Electron keeps its login token in the host.

SSE has bounded notifications and no durable replay: reload on `ready` (including reconnect) or `resync`. Placement/deletion changes send `resync` without IDs; notifications revalidate session. Message events carry public IDs and internal sequences, never bodies. Vue normalizes HTTP/Electron DTOs; native raw MIME crosses IPC as Buffer then Uint8Array.

IPC channels: `sandpost:inboxes:list/create`, `sandpost:views:list/save/delete`, `sandpost:messages:list/get/raw/search/attachment/delete/move/copy/trash/seen/flags` (list accepts `viewId`/DSL `query`), `sandpost:folders:list/create/update/delete/messages`, `sandpost:trash:messages/empty`, `sandpost:admin:scopes:*`, `sandpost:admin:users:*`, `sandpost:admin:smtp:server:get/save`, `sandpost:admin:smtp:accesses:list/create/regenerate/update/delete`, `sandpost:admin:query:validate`, `sandpost:imap:server`, `sandpost:imap:accounts:list/get/create/update/delete/regenerate/restore`, and `sandpost:imap:folders:create/update/delete` (caller-owned accounts). Each suffix is a separate channel validated in main (IDs, roles, addresses, usernames, query lengths). Credential inputs never carry passwords; IMAP inputs are allowlisted; preload has no general invocation. Subscriptions use `sandpost:event`, without another domain event bus.

Electron production serves Vue through `sandpost://app` with local SPA fallback; renderer operations never use runtime HTTP, which remains available to external tools. NativeRuntime applies shared environment settings and defaults desktop data to Electron's platform user-data path. Main starts it before a window, retains it when macOS windows close, and awaits shutdown before quit. `wait` propagates listener failures; `nextEvent` awaits Rust broadcast without polling, lag becomes `resync`, and cancel/shutdown resolves pending receives with null. Preload subscribes after attaching listeners, signals ready after acknowledgement, and removes listeners on unsubscribe.

The persistent Vue shell has Mail (Inbox, Trash, address inboxes, Smart Folders, manual Folders) and admin pages; its + menu creates a Smart Folder, address inbox, or manual folder. Manual folder deletion identifies exposed accounts and confirms messages return to fallback. Messages can move to folders/Inbox/Trash; Trash offers Empty Trash. Users/Scopes/SMTP appear only to Owners/Admins; IMAP appears to all users. Footer shows event state only while connecting/offline and offers appearance/sign-out. Narrow screens use a drawer; Cmd/Ctrl+B toggles it on desktop.

Folder/search are encoded in `/mail?folder=<view id>&q=<search>` and preserved on `/mail/<message id>`. The dense two-line list supports All/Unread, refresh, arrows and j/k; selection supports Shift ranges, Cmd/Ctrl-click, and Cmd/Ctrl+A over loaded rows. Delete/Backspace, swipe, header, and inspector actions move mail to Trash immediately; inside Trash they delete permanently after confirmation. Bulk DELETE uses bounded concurrency; partial failures remain selected for retry. Selection clears on folder/search change. Keyset paging loads near the end with an explicit “Load older messages” control. Inspector renders isolated HTML/plain text; Headers copies all or downloads `.eml`; Attachments downloads decoded MIME parts. Dialogs are accessible; narrow layouts switch between list and inspector.

Search spans readable mail; Cmd/Ctrl+K or `/` focuses it. It accepts canonical DSL, shortcuts, or text. Shortcuts: `from:`, `to:`, `cc:`, `subject:`, `has:attachment`, `after:YYYY-MM-DD`, `before:YYYY-MM-DD`; prefix `-` negates. They become canonical predicates (`from:x` matches `from.address` or `envelope.from.address`; dates use `received_at` bounds at local midnight); remaining words form one `content contains` literal. Quoted whole input is literal. Shortcuts are search-only; saved Views/scopes use DSL. Suggestions preview parsing/matches, Tab completes, arrows move, Enter opens/highlights or shows all, Escape closes then clears. Opening a result selects its receiving-address inbox or Inbox fallback.

The shared CodeMirror 6 editor serves Scopes, Smart Folders and Independent account smart folders. It shows line numbers, active line, validation/cursor status; highlights query words; completes grammar-valid fields/operators/boolean values/and/or, auto-closes quotes and `header[""]`, and supports Tab/Enter acceptance, Escape, and hover documentation. Server validation remains authoritative; errors underline the reported position, collection `!=` warns, and a collapsible reference lists fields/operators/examples. Smart Folders are named canonical-query Views: personal by default; Owners/Admins may share. Views never grant access.

Admin pages: `/admin/scopes` shows hierarchy, inherited filters, live newest-match preview, sibling order, assignments (owners omitted; All-mail users flagged). `/admin/smtp-access` edits the global listener address (immediate rebind), displays status/AUTH and credentials, and supports copy username, rotate, enable/disable, confirmed delete. Create asks name/optional username. One-time credential dialog shows host/port/user/password with copy, warns rotation invalidates old credentials, and cannot close until “I have saved my credentials” is checked; plaintext leaves component state and is never stored in browser storage/toasts. Users can be created/edited/deleted with role and All/Scoped access; editing signs the account out.

SMTP Access also exposes “Authentication settings” for TLS/mechanism rules and warns when TLS-only access lacks a certificate.

`/imap` lists the caller's accounts with Mirror/Independent mode and supports create,
configure, password rotation, enable/disable and confirmed deletion. `/imap/<id>`
shows connection setup, non-destructive mode switching, mailbox previews and account-local
smart folder CRUD only in Independent. Mirror shares primary organization; Independent
preserves local folders, read state and deletions. Mode transitions require client
reconnection; deleting a credential permanently removes its Independent state.
One-time credentials remain ephemeral. Notices explain disabled service, clear-text
password risk and manual self-signed certificate trust. Full behavior: [imap.md](imap.md).

Default Inbox is virtual with nil UUID; compatibility inboxes are Views for normalized recipients under `@sandpost.local`. Search indexes normalized facts in Tantivy as a disposable candidate index; canonical verification and current SQL revision/authorization checks prevent stale hits from exposing changed mail. Short substrings/negative queries may scan broadly and indexing is eventual. See [query-language.md](query-language.md), [scope-engine.md](scope-engine.md), and [storage.md](storage.md) for those contracts. Read status is authoritative primary server state, shared with Mirror accounts; Independent read state stays local to its account. Inbox badge counts newest-first until 1,000 unviewed or end: exact through 999, then `999+`, hidden at zero; it updates on opens, events and deletions. Appearance follows system until changed, then persists per device. Roles: Owner full authority; Admin manages users (except owners), scopes, SMTP access, shared Views and search maintenance; Member reads/deletes visible mail and keeps personal Views; Viewer reads visible mail and keeps personal Views. All mail access reads all mail; Scoped reads assigned scopes and inherited filters, with personal filters further restricting; no assignments means no mail. Owners bypass visibility restrictions. There is no outbound delivery; IMAP reads captured mail through users' accounts.

## Verify

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
python3 scripts/smoke.py
yarn frontend:build
yarn frontend:test
yarn desktop:check
yarn desktop:test
yarn native:build
yarn native:test
yarn desktop:smoke
yarn desktop:smoke --dev
python3 scripts/smoke.py --frontend apps/frontend/dist
```

`cargo test --workspace --locked` and `yarn frontend:test` run the suites; the root smoke starts the binary with isolated ports/data, sends SMTP, checks MIME, HTTP authorization/search/raw/SSE, then uses independent Python `imaplib` for login, LIST/SELECT/FETCH/SEARCH/STORE/IDLE push/MOVE and verifies restart persistence, UIDVALIDITY, UIDs and flags. `--frontend apps/frontend/dist` additionally checks assets, deep SPA routes and API 404 boundaries.

`yarn desktop:check`/`desktop:test` cover main/preload typing and IPC/security. `yarn native:test` after addon build covers real SMTP, events, reads, Buffer fidelity, HTTP parity, cancellation, shutdown/restart. Electron smoke sends real MIME and checks notifications, list/detail/raw IPC, exact bytes, Vue UI, HTML isolation and blocked HTTP API. Production uses the local app protocol; dev permits Vite/HMR but blocks `/api`. For unpacked app use `yarn desktop:smoke --packaged <absolute executable path>` (macOS executable: `SandPost.app/Contents/MacOS/SandPost`).

`yarn desktop:smoke --preview` (or `yarn workspace @sandpost/desktop smoke:preview`) exercises inline inspection through SMTP, preload, Vue and Chromium; add `--dev` or `--packaged <absolute executable path>`. It checks real DOM/inline CSS, blocked script/Node/preload/storage, exclusive DevTools target, one window, resize bounds, themed first frame/readiness loader, live/reopened light/dark theme, toolbar close/reopen, stable normal/malformed HTML inspection, minimum size, and teardown on Plain text/unmount. Because MIME may provide HTML for text/plain, it explicitly selects Plain text mode. It checks Elements selection, computed style, box model, minimize/restore. `SANDPOST_PREVIEW_CAPTURE=1` saves DevTools/email captures and loader frame in the system temp directory. Frontend/desktop tests also cover browser fallback, lifecycle, theme/toggle races, clipping/geometry, IPC policy/zoom, destruction and cleanup.

Manual inspection: test normal and malformed styled/table HTML; Inspect, select via native picker, inspect Styles/Computed/box model and Console/Sources/Network. Drag divider to limits; resize/zoom/minimize/restore; close/reopen; change HTML while open; switch Plain text/Headers, routes and folders; open menus/dialogs and scroll to verify clipping/restoration. Repeat with scripts, handlers, remote images, forms, popups and navigation and confirm they stay blocked. Browser preview has no Inspect.

Local migration validation covered macOS ARM64 native debug/release, Vite dev, production assets and unsigned `.app`. Windows, Linux and macOS x64 are configured CI targets, not locally verified releases. Installer images, signing and notarization are unvalidated.

## Architecture and operational limits

- [Architecture and component boundaries](architecture.md); [storage contract and SQLite migrations](storage.md); [query grammar and semantics](query-language.md); [scope authorization and index synchronization](scope-engine.md); [IMAP behavior and limits](imap.md); [code style](code-style.md).
- Implemented: one global authenticated SMTP catcher and mail pool; canonical users/roles/mail access; hierarchical scopes; personal/shared Views; SQLite storage contract and backend; typed DSL; durable search outbox/disposable Tantivy generations; MIME normalization; HTTP/SSE; optional SPA; shared browser/mock/Electron clients and embedded runtimes; users' IMAP accounts with IMAP4rev1 IDLE, UIDPLUS, MOVE, UNSELECT, NAMESPACE, ID, CHILDREN, LITERAL-, SASL-IR, STARTTLS/implicit TLS, virtual mailboxes and persistent UID state; manual folder exposure and per-message placement.
- Limits: search sync is eventual (10 ms poll, 50 ms quiet, max five-second wait; shutdown drains); broad negative/short substring searches may scan widely. Outbound mail, durable SSE replay, disk-backed attachments, advanced mail-management features are absent. IMAP APPEND, server Drafts/Sent, CONDSTORE/QRESYNC, LIST-EXTENDED and IMAP4rev2 are absent; IMAP SEARCH scans selected mailbox rather than Tantivy. Apple Mail/Thunderbird/Outlook lack automated GUI verification. SMTP TLS requires configured cert/key (no generation, upload or hot reload); without it SMTP AUTH is clear text. HTTP has no built-in TLS.
- SQLite uses one ordered script-name migration sequence (no schema versions/batches); startup validates applied prefix and physical schema, then atomically applies missing scripts/history. Append future scripts. Old pre-deployment batch-history/pre-release databases are unsupported; recreate them, as startup rejects without clearing data. No active message-to-scope materialization; authorization composes mail access, assignments/inherited filters and personal filters. Mirror exposes every primary normal/smart folder; Independent initializes primary normal organization once and owns its own smart definitions. Smart results and global search include filed mail and exclude primary Trash. Reference-safe collection releases physical content only after all views release it.
- Search status/rebuild are administrator HTTP operations and offline CLI commands. HTTP rebuild allows SMTP ingestion; CLI status reads SQL without writer lock, CLI rebuild requires the installation lock. Index schema/checksum validation precedes recovery; indexing worker failure stops supervised listeners. In-flight work can finish with already-read facts; later requests re-read roles/policies. Credential-bound sessions are invalidated on password change, and administrators cannot grant authority they lack. SMTP server edits rebind/re-evaluate AUTH without restart; an environment override lasts until process restart without it.
- Refresh waits for its captured committed index watermark, then rechecks visibility/session before IDs; SSE `ready` also waits. SMTP acknowledgement depends on SQL commit and direct reads remain immediately available. Shutdown releases index locks even if handles survive; closed writers reject mutations/wake waits. Temporary coordinators fence rebuild against outbox pruning and retry or fail under sustained acknowledgement churn.
- Administrators cannot edit credentials or delete users whose authority exceeds their own; this prevents privilege escalation by credential reset. Owners can manage all accounts.
