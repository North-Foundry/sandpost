# IMAP

Sandpost serves captured mail through user-owned IMAP credentials. Physical message
content is shared; the owner’s current mail access, scopes, role and personal filter
bound every account. Materialized memberships and retained references never grant access.

## Accounts and organization modes

Accounts have `mode: mirror | independent`. New accounts default to Mirror; old
credentials migrate to Independent to preserve their former account-local organization.
Legacy `mirror_views` remains an API compatibility input only: when no explicit mode
is supplied, true selects Mirror and false selects Independent. Linked View selections
are retained as compatibility data; Independent does not inherit primary smart Views.

Mirror exposes the primary user’s current normal folders, Trash, smart Views, flags,
COPY memberships, and permanent removals. Folder CREATE/RENAME/DELETE and message
COPY/MOVE/Seen updates act on the same primary state used by Web and every Mirror
credential. Independent owns its folder paths, placements, copies, flags, Trash,
retained history and smart definitions. Web and other credentials never overwrite it.

Creating any credential snapshots the primary user’s currently authorized, untombstoned
history, normal organization and flags into its dormant Independent state. Primary
smart definitions are not copied. Every later arrival adds an Independent reference
even while the credential is in Mirror. A new credential never recovers primary-deleted
mail retained only by an older Independent account.

Mode changes preserve Independent folders, memberships, flags and mailbox identities.
Each mode has distinct mailbox identity/UID state. A monotonic session generation advances on mode changes, disabling and password rotation; it revokes
existing sessions with BYE at validation/IDLE refresh, including a switch away and back
between commands. Clients reconnect and SELECT the active mode’s UIDVALIDITY. Dormant
state does not bypass changed owner permissions. Session mutations supply the expected generation
and validate it inside their storage transaction, including an away-and-back switch.

A credential has a generated password shown only on issuance; only Argon2id hashes
are stored. Disable, password rotation and owner/account deletion revoke sessions.
One user may own 25 accounts; management is caller-owned, including for administrators.
Deleting a credential cascades both mode catalogs and its Independent references,
folders, flags, smart definitions and subscriptions. Primary organization survives.
The UI requires destructive confirmation.

## Normal folders and smart mailboxes

Normal folders support `/` hierarchy paths. Explicit parent renames rename descendants
transactionally while retaining folder IDs and UIDs. LIST reports HasChildren and
nonselectable implicit parents. DELETE removes the named folder, retaining children;
messages whose only normal occurrence belonged to it enter the relevant Trash.
Other normal copies survive and may become primary placement. INBOX and Trash are
protected fixed mailboxes. Smart folder names remain flattened and collision-resolved.

Mirror lists primary personal/shared Views and legacy owner smart definitions. Independent
smart definitions are managed in that credential’s Web settings through `/folders`.
They evaluate its retained, currently authorized messages independently of normal
placement. Filing a message never excludes it from a matching smart result. Primary
and account Trash are excluded from ordinary smart results; explicit Trash-inclusive
queries are not currently supported by the facts-only query language.

Smart SELECT is writable only for Seen: PERMANENTFLAGS reports `(\\Seen)`;
STORE of other flags, MOVE from smart results, COPY/MOVE into smart results and
EXPUNGE are refused. FETCH may set Seen; EXAMINE remains fully read-only. Seen-only
STORE replacement changes Seen without clearing other underlying flags. COPY out of
a smart result preserves its dynamic membership. Some clients may not expose read
updates for restricted virtual mailboxes.

## Membership, UIDs and flags

SQL membership assigns ascending UIDs in physical-message insertion order. Existing
members keep UIDs; reentry receives a higher UID. Each identity receives UIDVALIDITY
from a persistent increasing allocator. Filters/authorization are canonicalized and
fingerprinted; changes reconcile membership transactionally. Source identities belong
to credential, mode, and source. A stale catalog omission never deletes a still-valid identity;
actual definition deletion cascades the identity, while hidden sources cannot pass the live catalog. Each mailbox contains at most one occurrence per
physical message; COPY/MOVE to a destination already containing it is refused atomically
with CANNOT instead of reusing a UID. UIDNEXT, sequence numbers and COPYUID stay mailbox-local.

Seen, Answered, Flagged, Draft and bounded keywords are shared per physical message
within the active view. Mirror uses primary user flags; Independent uses account flags.
Deleted remains a reversible marker on one mailbox occurrence. Web COPY preserves sources;
primary flag editing supports the same non-Deleted flags and bounded keywords. Web Seen updates and
opening a Web message use the primary state and notify Mirror clients. Read-state
changes are reflected in Web summaries and unread counts.

## Deletion and retention

Web Delete moves an underlying message to primary Trash, removing its normal placements
and copies. Delete in Trash and Empty Trash permanently release the primary view.
Legacy explicit permanent-deletion API calls remain permanent. Deleting a Web smart
result uses the same underlying Trash action, never a query exclusion.

IMAP STORE Deleted changes only the selected occurrence. EXPUNGE/CLOSE/UID EXPUNGE
remove flagged occurrences in the active view: additional copies disappear independently;
a surviving copy is promoted if the primary occurrence is removed. A final normal
occurrence enters Trash; expunging the final Trash occurrence permanently releases the
view. COPY followed by source EXPUNGE transfers organization for clients without MOVE.
MOVE never tombstones the view. Independent EXPUNGE never deletes primary/peer state.
This Trash-retention policy extends the product beyond mailbox-local protocol deletion;
protocol responses still remove the source occurrence and preserve UID mappings.

Physical content survives every retained Independent reference, including dormant and
currently unauthorized state. Primary users conservatively retain untombstoned mail,
including temporarily inaccessible scopes. After every primary user has tombstoned a
message and all Independent references disappear, deletion/account cleanup collects it
transactionally. Foreign keys remove facts, attachment metadata and memberships; the
existing durable outbox queues search deletion. Physical raw MIME contains attachments,
so deleting the parent removes attachment bytes too. Conservative primary retention can
delay collection until another primary user releases a message. Privileged physical
maintenance methods remain separate from user-facing logical deletion.

## Authorization, subscriptions and synchronization

Each command reloads credentials and current owner authorization. FETCH, SEARCH, SELECT,
COPY/MOVE, smart queries and IDLE refresh reconcile against authoritative facts. Previously
issued UIDs and mode switches never grant access. Permission loss removes visible
memberships while preserving account organization internally; reauthorization may reenter
with new higher UIDs.

Subscription names survive deletion/renaming; absent subscribed names appear in LSUB
with Noselect and cannot SELECT. Default subscription observations never overwrite
explicit unsubscribe exceptions. Mode-specific catalogs share credential subscriptions.

Post-commit primary mutations publish Visibility; Independent mutations publish the
credential’s change. Physical and permanent deletion publish Mail. New arrivals use
MessageEvent. Both HTTP SSE and the native desktop event stream turn committed changes
into resync hints; open message lists and unread counts reload automatically after
Mirror Seen changes, including reads in external IMAP clients. Native hints revalidate
the session and remain cancellable without polling. IDLE coalesces updates for 50 ms and then rereads state, without polling;
failed/timed-out refresh ends the connection with BYE UNAVAILABLE. Foreign EXPUNGE
responses are deferred during sequence-based FETCH/STORE/COPY/MOVE and SEARCH to
preserve sequence targets. SQLite immediate transactions serialize mutation, UID
allocation, folder rename/delete, mode changes and garbage collection.

APPEND/import is unsupported and returns protocol errors. COPY/MOVE of existing mail
remain supported. The operation matrix and original audit are in
[imap-operation-matrix.md](imap-operation-matrix.md).

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

All routes require a session. Account routes operate on the caller's accounts; folder routes manage the active mode’s smart definitions:

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

Web manual folders use `GET/POST /api/v1/folders`, `PUT/DELETE /api/v1/folders/<id>`, and `GET /api/v1/folders/<id>/messages`. Requests carry `{ name }` (legacy `exposedAccounts` is accepted); responses include owner and selected account IDs. `POST /api/v1/messages/placement` accepts `{ identifiers, folderIdentifier }` (null restores baseline), at most 256 messages. Exposure grants no authorization. The deletion dialog explains moving otherwise unfiled messages to Trash.

SEARCH content criteria scan the selected mailbox's messages rather than Tantivy; full reconciliation after a definition change scans the mail pool once in SQL. There is no CONDSTORE/QRESYNC, LIST-EXTENDED, or IMAP4rev2; normal folder paths support hierarchy and clients cannot upload drafts or sent copies. SQLite writes serialize through one connection, with four read-only connections under WAL; in-memory storage uses one connection for reads and writes. See [storage.md](storage.md) for persistence and migration contracts.

IMAP4rev1 content FETCH refuses captured messages containing NUL with `NO [CANNOT]`; metadata-only requests remain available and captured octets are unchanged. FETCH and SEARCH share a MIME tree bounded to depth 32 and 2,048 entities. Composite content exceeding those bounds, or an embedded message requiring transfer decoding before section addressing, becomes an opaque `application/octet-stream` leaf. Raw FETCH remains available, while nested sections and body searches inside that fallback are unavailable.
