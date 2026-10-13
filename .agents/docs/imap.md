# Sandpost IMAP accounts and virtual mailboxes

Sandpost runs one installation-wide IMAP service. Every user, regardless of global role, can own multiple personal accounts and connect an IMAP4rev1 client. IMAP mailboxes are virtual views over the single captured-mail pool: an account never reads beyond its owner's current Sandpost authorization.

## Components

```text
sandpost-imap protocol (framing, session, rendering)  crates/sandpost-imap
    | ImapBackend / Account / ChangeWatch
runtime adapter                                       crates/sandpost/src/imap_backend.rs
    | ImapService / ImapAccountSession
application IMAP (accounts, catalog, auth, changes)   crates/sandpost-application/src/imap/
storage contract                                      crates/sandpost-storage/src/imap.rs
SQLite implementation                                crates/sandpost-storage-sqlite/src/imap/
```

The protocol crate owns sequence numbers, `\Recent`, deferred EXPUNGE, MIME sections, ENVELOPE, BODYSTRUCTURE, and SEARCH evaluation. The application owns credentials, account ownership, mailbox planning, authorization, and notifications. Storage owns identities, UIDs, membership, and flags. See `crates/sandpost-imap/RFC-COVERAGE.md` for protocol coverage.

## Accounts and credentials

An account stores identifier, Sandpost owner, display name, unique installation-wide username, Argon2id password hash, enabled and mirror settings, timestamps, and last use. Generated passwords are 32 characters (about 185 bits), use the SMTP credential helpers in `sandpost-application/src/credentials.rs`, and are returned only on create/regenerate with HTTP `Cache-Control: no-store`. Listings expose neither password nor hash; credentials are not logged.

Authentication verifies hashes off the async executor, with at most four concurrent verifications and a dummy hash for unknown usernames. Disabled accounts and accounts with missing owners are refused. Throttling blocks verification after 20 failures per client address or 50 per username in 15 minutes (`NO [UNAVAILABLE]`); the protocol adds a two-second delay per failure and closes after three failures.

Every post-login command revalidates that the account exists, remains enabled, belongs to the same existing owner, and retains the verified hash. Disable, password regeneration, or account/owner deletion ends sessions with `BYE`, including IDLE sessions. A user may own at most 25 accounts. Management is caller-owned only; another user's account is not found even for an Owner or Admin.

## Mailboxes and authorization

LIST order is:

1. `INBOX`: authorized general fallback, unless inherited Web or account placement files mail elsewhere.
2. `Trash` (`\Trash`): messages explicitly copied or moved there.
3. Linked Sandpost Views: all visible personal/shared Views when mirroring is enabled, otherwise selected still-visible Views; aliases may rename them.
4. Legacy owner-shared smart folders (maximum 100): canonical-query filters managed through any owner account. They are separate from Views. One owner definition is authoritative; account projections retain identity/UID state, edits update all projections, credential deletion removes only its projection, and new credentials project existing folders. Canonical definitions and projection persistence are described in [storage.md](storage.md).
5. Web manual folders explicitly exposed to the account. Web owns definitions/exposure; only that account organizes their IMAP content.
6. Account-private manual folders. CREATE/RENAME/DELETE affect only these; deleting one never deletes mail.

Names are derived on each request from alias, View, or folder name. `/` and controls become `-`; sorting and collision checks are case-insensitive. Collisions (including `INBOX`/`Trash`) gain ` (2)`, ` (3)`, etc. Renaming a View preserves mailbox identity and UIDVALIDITY.

Authorization, organization, and deletion are separate. Account placement wins over exposed Web placement, then smart/filter baseline. Moves do not alter another account, Web placement, or scopes. INBOX is the fallback for all authorized unfiled mail, including with no selected smart folders. COPY into a manual folder adds account-local membership; MOVE of a baseline or primary occurrence sets exclusive account placement; moving an additional COPY occurrence relocates only that copy.

Each mailbox definition composes the owner's current `authorization::compose` result (mail access, assigned scopes with ancestors, personal filter; no restrictions for owners) with its own filter:

| Mailbox | Definition |
| --- | --- |
| INBOX, mirroring | `true`: all mail the owner can read, like Sandpost Inbox |
| INBOX, selected Views | General authorized fallback, independent of smart-folder selection |
| Trash | `true`, with explicit copied/moved membership and current-authorization check |
| Linked View | View's current filter |
| Local folder | Folder's filter |

A stored filter that no longer compiles matches nothing. Definitions are canonicalized and fingerprinted with `Expression::into_canonical_with_fingerprint`.

## Membership, UIDs, and flags

Membership is materialized in SQL (`imap_mailbox_messages`), independent of search positions/index results. `synchronize_imap_mailbox` runs transactionally: a changed fingerprint fully removes nonmatching members; dynamic mailboxes add matching messages above the last examined sequence (or scan all after definition change), excluding existing and expunged members, with ascending UIDs in message order. Trash and COPY targets restore membership only while authorized. With an unchanged definition and no new mail, synchronization only reads. Filters use the shared SQL compiler and authoritative facts.

UIDs are never renumbered or reused; re-entry gets a higher UID. Each mailbox identity gets UIDVALIDITY from a persistent, always-increasing allocator (preferring current time); an existing identity retains it, while deletion/recreation gets a new value. `\Recent` uses a per-mailbox watermark claimed by a read-write SELECT or synchronization.

| Change | Result |
| --- | --- |
| New mail matches a dynamic mailbox | Added at next synchronization with next UID; IDLE clients receive EXISTS/RECENT. |
| Filter, scope, personal filter, role, or mail access stops matching | Membership removed; clients receive EXPUNGE; other UIDs stay fixed. |
| Definition change makes mail match | Added with a new higher UID. |
| Sandpost deletion | Membership cascades away and clients receive EXPUNGE. |
| View unexposed (deleted, unselected, mirroring off) or local folder deleted | Identity and members deleted; selected sessions get EXPUNGE for each message; LIST removes it. Re-exposure creates a new identity/UIDVALIDITY. |
| View/folder renamed | Identity and UIDVALIDITY persist. |
| Account disabled, password changed, or account/owner deleted | Sessions receive BYE; disabled accounts cannot log in. |

`\Seen`, `\Answered`, `\Flagged`, `\Draft`, and keywords (maximum 32 per message, 64 characters each) are per account/message, so Seen is shared across that account's mailboxes. `\Deleted` is a removable per-mailbox-membership flag until expunge. An account-wide modification counter is the change cursor.

## COPY, MOVE, expunge, and subscriptions

IMAP never physically deletes Sandpost mail. EXPUNGE, CLOSE, and UID EXPUNGE tombstone flagged messages for the owner; a database trigger removes them from all that user's accounts/mailboxes and reconciliation/COPY cannot restore them. Other users' access is independent. Genuine MOVE updates membership without a tombstone and is the non-deleting transfer. New COPY/MOVE destination memberships, including Trash, receive fresh ascending UIDs and COPYUID mappings; COPY preserves the source.

COPY/MOVE into any smart folder is refused with `NO [CANNOT]`, even for matching mail. CREATE makes an empty account-private folder; RENAME/DELETE cannot target Web folders, Views, legacy smart folders, or Trash (`NO [CANNOT]`). Each mailbox holds at most one occurrence per physical message: a destination that already has it, including the same mailbox, refuses COPY/MOVE atomically instead of reusing a UID. Moving an additional COPY changes only that occurrence; primary placement and other copies remain. Explicit subscriptions survive deletion/exposure removal. APPEND is unsupported and hierarchy is flat.

Account placements survive filter/scope changes but do not grant access. Deleting a destination folder or removing its exposure clears placements targeting it and restores filtered/INBOX fallback. Tombstones persist. “Restore hidden messages” clears legacy account-local exclusions only, not placement overrides or tombstones.

COPY followed by `STORE +FLAGS (\Deleted)` and EXPUNGE follows standalone-expunge policy and deletes for the owner. Clients must use MOVE for a non-deleting transfer.

Subscriptions are per account and stored by mailbox name. Live mailboxes default to subscribed unless explicitly unsubscribed. Listing records newly observed defaults without overwriting unsubscribe exceptions, including concurrent UNSUBSCRIBE. Default observations and explicit names survive delete/rename/unexposure: LSUB returns the old name with `\Noselect`, while LIST contains current names only. Retained names cannot SELECT/STATUS absent mailboxes. UNSUBSCRIBE removes the name; recreating/exposing it makes it selectable again without duplicate listing. Migration cannot infer names deleted before retention existed; defaults become durable when live names are first listed after upgrade.

## Changes and IDLE

After commit, `Application` publishes `Mail` (message deletion), `Visibility` (users, roles, mail access, scopes, assignments, personal filters, Views), and `ImapAccount(identifier)` (settings, credentials, linked Views, folders, flags, expunges, copies). New mail uses the existing post-commit `MessageEvent`. `ImapChangeWatch` merges both channels and ignores other accounts' IMAP events; lag counts as a change. IDLE notifications coalesce for 50 ms, then trigger revalidation and synchronization; there is no polling. Every non-IDLE command first synchronizes the selected mailbox.

## Configuration

| Variable | Default | Meaning |
| --- | --- | --- |
| `SANDPOST_IMAP_LISTEN` | `127.0.0.1:1143` | Listener address; `off` disables IMAP |
| `SANDPOST_IMAP_TLS` | `auto` | With certificate, TLS; STARTTLS required off-loopback. Without one, clear text only on loopback. `required` always needs a certificate; `off` permits clear-text passwords on any address (explicit opt-in). |
| `SANDPOST_IMAP_TLS_MODE` | `implicit` | `implicit` (IMAPS, usually 993) or `starttls` (usually 143) |
| `SANDPOST_IMAP_TLS_CERTIFICATE`, `SANDPOST_IMAP_TLS_PRIVATE_KEY` | SMTP pair | PEM chain/key; set both or neither |
| `SANDPOST_IMAP_HOST` | Bound address | Host shown in connection instructions |
| `SANDPOST_IMAP_MAX_CONNECTIONS` | `64` | Concurrent sessions, 1–10000 |

`auto` without a certificate on a non-loopback address stops startup rather than accepting clear-text passwords; bind failure also stops startup. Startup prints IMAP host, port, security, and address, or `Status disabled`. Certificates load at startup; there is no certificate generation, and self-signed certificates must be trusted by the client or OS.

## Management API and limits

All routes require a session. Account routes operate on the caller's accounts; folder routes manage owner-shared folder definitions through one of them:

```text
GET /api/v1/imap/server
GET/POST /api/v1/imap/accounts
GET/PUT/DELETE /api/v1/imap/accounts/<id>
POST /api/v1/imap/accounts/<id>/password
POST /api/v1/imap/accounts/<id>/restore
POST /api/v1/imap/accounts/<id>/folders
PUT/DELETE /api/v1/imap/accounts/<id>/folders/<folder>
```

Folder filters compile before save and linked Views must be visible. Request/response fields are in [project.md](project.md).

Web manual folders use `GET/POST /api/v1/folders`, `PUT/DELETE /api/v1/folders/<id>`, and `GET /api/v1/folders/<id>/messages`. Requests carry `{ name, exposedAccounts }`; responses include owner and selected account IDs. `POST /api/v1/messages/placement` accepts `{ identifiers, folderIdentifier }` (null restores baseline), at most 256 messages. Exposure grants no authorization. The deletion dialog lists affected account names and says mail is retained.

SEARCH content criteria scan the selected mailbox's messages rather than Tantivy; full reconciliation after a definition change scans the mail pool once in SQL. There is no CONDSTORE/QRESYNC, LIST-EXTENDED, or IMAP4rev2; folders are flat and clients cannot store drafts or sent copies. SQLite writes serialize through one connection, with four read-only connections under WAL; in-memory storage uses one connection for reads and writes. See [storage.md](storage.md) for persistence and migration contracts.
