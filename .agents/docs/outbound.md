# SMTP Outbound, relay and forwarding

Sandpost is a sandbox: captured mail stays inside it unless an Owner explicitly configures
external delivery. Two separate concepts make that possible.

- An **SMTP Outbound** is a reusable, instance-level connection to an external SMTP server
  (host, port, TLS mode, credentials, timeouts, recipient safety allowlist). It decides *how* to
  reach a server, never *which* mail is delivered. One SMTP Outbound can serve any number of
  scopes and routing actions.
- A **routing action** belongs to one scope and references one SMTP Outbound. It decides *what*
  is delivered:
  - **Relay** delivers the message to its original SMTP envelope recipients (`RCPT TO`, Bcc
    included), never to addresses reconstructed from `To`/`Cc` headers.
  - **Forward** delivers a copy to the action's configured recipients only; the original
    recipients receive nothing unless they are listed.

A scope may hold any number of actions, including several of the same type. Relay and forward
share one delivery engine and differ only in how the envelope recipients are chosen.

## Example: Pippo and Pluto

| Scope | Action | SMTP Outbound | Recipients |
| --- | --- | --- | --- |
| Scope A | Forward | Pippo (`smtp.example.com:587`, STARTTLS, PLAIN) | `qa@example.com` |
| Scope B | Forward | Pluto (`smtp.other.com:465`, implicit TLS, LOGIN) | `audit@example.com` |
| Scope B | Relay | Pippo | the message's original envelope recipients |

1. Under **Administration → SMTP Outbound**, create *Pippo* and *Pluto*, enter their credentials,
   allow the recipients they may reach (for example `*@example.com`), enable them and use
   **Test connection**.
2. Under **Administration → Scopes**, select Scope A and use **Routing → Add forward** with Pippo
   and `qa@example.com`; select Scope B and add a forward through Pluto and a relay through Pippo.
3. Mail whose most specific matching scope is Scope A is forwarded to `qa@example.com` through
   Pippo. Mail in Scope B is forwarded to `audit@example.com` through Pluto and relayed to its own
   recipients through Pippo, so Pippo serves both scopes with one configuration.

The same through the API (Owner session):

```http
POST /api/v1/smtp/outbounds
{ "name": "Pippo", "host": "smtp.example.com", "port": 587, "security": "starttls",
  "auth_type": "plain", "username": "relay", "password": "…", "enabled": true,
  "allowed_recipients": ["*@example.com"] }

POST /api/v1/scopes/<scope-a>/routing-actions
{ "smtp_outbound_id": "<pippo>", "action_type": "forward", "forward_recipients": ["qa@example.com"] }

POST /api/v1/scopes/<scope-b>/routing-actions
{ "smtp_outbound_id": "<pippo>", "action_type": "relay" }
```

## Which scope routes a message

Scopes are query definitions, so a message belongs to every scope whose filter, together with all
its ancestors' filters, it satisfies. Routing uses only the **most specific** of those scopes: a
matching scope that has a matching child yields to the child. Actions are **explicit**: an action
applies to mail whose most specific matching scopes include its own scope, and it is never
inherited by child scopes. Mail matching two sibling branches gets the actions of both. Scope
assignments, mail access and personal filters are unrelated to routing: they decide who can read
mail, not where it is delivered.

Inheritance is deliberately absent. `RoutingConfiguration::applicable_scopes` in
`sandpost-application` is the single place where a future policy could add ancestors; delivery
does not depend on it.

## Delivery pipeline

1. SMTP accepts and parses the message.
2. `MessageStorage::insert_message` stores it and, in the same SQL transaction, queues a row in
   `mail_routing_evaluations`, but only while at least one enabled routing action uses an enabled
   SMTP Outbound. Without routing configuration nothing is written and capture behaves exactly as
   before. SMTP acknowledges after the commit and never waits for an external server.
3. The outbound dispatcher (`Application::run_outbound`, started by the runtime beside the
   listeners and outside transport supervision) wakes on message notifications, configuration
   changes and a one-second poll. It evaluates queued messages against the current scopes and
   enabled actions and creates one delivery job per applicable action, atomically with removing
   the queue entry (`complete_routing_evaluation`; a job is unique per message and action).
   Mail captured before an action existed is never routed retroactively.
4. Due jobs are claimed (`processing`) and attempted as independent tasks, at most
   `concurrent_deliveries` at once and at most `max_connections` per SMTP Outbound. Each attempt
   rereads the action, the SMTP Outbound, its password and the message, so a disabled or deleted
   action or outbound stops queued deliveries.
5. The outcome is recorded per recipient and per attempt; the stored message is never modified
   and stays readable whatever happens to its deliveries.

Delivery never changes how mail is displayed, searched or indexed.

### What is sent

The engine (`sandpost-smtp-outbound`) re-submits the captured bytes with three adaptations: the
capture's `Return-Path` is removed (a relay must not send one), bare line feeds become CRLF, and
when **override From** is set the `From` field is replaced and the original kept in
`X-Original-From`. Everything else, including MIME structure, attachments, `Received` trace fields
and the absence of `Bcc`, is unchanged; Bcc recipients exist only in the envelope. The reverse path
is **envelope From** when set, otherwise the original SMTP `MAIL FROM` (the null path `<>` stays
null). 8-bit content is sent with `BODY=8BITMIME` and refused permanently by a server that does not
announce 8BITMIME. Every recipient gets its own `RCPT TO`, so each receives its own outcome, and the
data goes only to accepted recipients. `EHLO` uses **HELO name** or `sandpost.localhost`.

### Statuses, retries and failures

Jobs move through `pending`, `processing`, `retrying`, `delivered` and `failed`; recipients are
`pending`, `delivered` or `failed`; attempts record `delivered`, `partial`, `deferred`, `failed` or
`interrupted` with counts and the session-level error.

| Failure kind | Example | Retried |
| --- | --- | --- |
| `connection` | DNS, refused connection, timeout, dropped connection | yes |
| `tls` | handshake or certificate failure, STARTTLS not offered | yes |
| `temporary` | 4xx reply (per recipient or for the whole transaction) | yes |
| `authentication` | 535 rejected credentials, no usable AUTH mechanism | only 4xx replies |
| `permanent` | 5xx reply, invalid address, 8-bit data without 8BITMIME | no |
| `policy` | non-public destination, recipient outside the allowlist, mail loop | no |
| `configuration` | action or outbound disabled/deleted, password unusable | no |
| `interrupted` | the process stopped during an attempt | yes |

Retries use exponential backoff: `retry_delay_seconds` (default 60) after the first attempt,
doubling each time, capped at one hour, for at most `maximum_attempts` (default 8). After the last
attempt, recipients still deferred fail with "gave up after N attempts". A delivery is `delivered`
when every recipient was delivered, `retrying` while any recipient is pending, and `failed`
otherwise, so a partially successful multi-recipient delivery is visibly failed while its
delivered recipients keep their own status. Owners can **Retry** a failed delivery: its failed
recipients become pending and the attempt budget restarts.

### Restarts and duplicates

SMTP cannot guarantee exactly-once delivery. Before dispatching, a starting process moves jobs
left `processing` back to `retrying` and records their running attempt as `interrupted`. The
interrupted attempt may already have been accepted by the server, so the retry can deliver a
duplicate; likewise a connection lost after the message data was sent is retried. Delivery is
therefore at least once for transient failures and never claimed to be exactly once. Shutdown stops
running attempts; their jobs resume after the next start.

## Security

- Nothing is delivered by default: there are no SMTP Outbounds or actions, new outbounds created
  through the API are disabled unless `enabled` is set, and every action needs an outbound.
- Only **Owners** manage SMTP Outbounds and routing actions and read deliveries; the application
  layer enforces this for HTTP and Electron alike (Members and Viewers receive 403, anonymous
  callers 401). Delivery history is additionally bounded by the Owner's own mail access, scope
  assignments, personal filter and deletions, so it never reveals unreadable mail.
- **Recipient allowlist:** each SMTP Outbound lists the recipient address patterns it may deliver
  to (`*` any run, `?` one character, ASCII case-insensitive, whole address). An **empty list
  permits nobody**; `*` permits everyone. Forward recipients outside the list are refused when the
  action is saved; relay recipients outside it fail with a `policy` error without being contacted.
  The list is checked again at every attempt.
- **Destinations:** the host is resolved once per connection and every resolved address must be
  public unless `[outbound] allow_internal_addresses` is enabled; the connection then uses only
  those addresses, so DNS answers cannot redirect it to loopback, private, link-local (cloud
  metadata), carrier-grade NAT or other non-public services. The rule reuses the link checker's
  address policy. Enable internal addresses for self-hosted relays on private networks or local
  development servers.
- **TLS:** STARTTLS, when configured, is mandatory: a server that does not offer it fails the
  attempt instead of continuing in clear text. Implicit TLS encrypts from the first byte. Certificates
  and host names are verified with the system trust store unless **TLS verification** is turned off
  explicitly. Plain SMTP is used only when chosen; the interface warns when credentials would travel
  unencrypted.
- **Credentials:** passwords are encrypted with ChaCha20-Poly1305 under a 256-bit key stored outside
  SQLite (default `<data directory>/outbound-secret.key`, created with owner-only permissions on
  first start; `[outbound] secret_key_file` / `SANDPOST_OUTBOUND_SECRET_KEY_FILE` move it). Each value
  is bound to its SMTP Outbound identifier. No API, IPC or log ever returns or prints a password;
  editing an outbound without a password keeps the stored one, and switching to no authentication
  removes it. Keep the key file with database backups: without it stored passwords cannot be
  decrypted and must be entered again. A database copy alone reveals no password; a copy of the whole
  data directory includes the key, so store the key elsewhere when that matters. Authentication
  exchanges are never logged (lettre's protocol tracing is not enabled).
- **Mail loops:** a message that already carries more than 30 `Received` fields is not routed again.

## Configuration

| TOML `[outbound]` | Environment | Default | Meaning |
| --- | --- | --- | --- |
| `allow_internal_addresses` | `SANDPOST_OUTBOUND_ALLOW_INTERNAL_ADDRESSES` | `false` | Permit non-public SMTP destinations |
| `concurrent_deliveries` | `SANDPOST_OUTBOUND_CONCURRENT_DELIVERIES` | `4` | Attempts running at once, 1–64 |
| `maximum_attempts` | `SANDPOST_OUTBOUND_MAXIMUM_ATTEMPTS` | `8` | Attempts before deferred recipients fail, 1–100 |
| `retry_delay_seconds` | `SANDPOST_OUTBOUND_RETRY_DELAY_SECONDS` | `60` | First retry delay, doubling up to one hour, 1–86400 |
| `secret_key_file` | `SANDPOST_OUTBOUND_SECRET_KEY_FILE` | `outbound-secret.key` | Password key file; relative paths resolve against the data directory |

Per SMTP Outbound: `name` (unique ignoring case), `host` (DNS name or IP), `port`, `security`
(`none`, `starttls`, `tls`), `auth_type` (`none`, `plain`, `login`), `username`, write-only
`password`, `enabled`, `override_from`, `envelope_from`, `connect_timeout` (1–300 s, default 10),
`smtp_timeout` (1–600 s per command, default 60; the message data may take up to ten minutes as RFC
5321 recommends), `tls_verify` (default `true`), `helo_name`, `max_connections` (1–16, default 2)
and `allowed_recipients` (at most 100 patterns). Forward actions take 1–50 recipients.

Deleting an SMTP Outbound that routing actions reference is refused (409) until those actions are
deleted or moved to another outbound; deleting an unreferenced one fails its waiting deliveries and
keeps their history with the recorded outbound name. Disabling an outbound or action, or changing an
action's type or outbound, fails its waiting deliveries; attempts already running finish. Deleting a
scope deletes its actions. Deliveries disappear with their message's physical deletion; the newest
10,000 finished deliveries and 50 attempts per delivery are kept.

## Interfaces

HTTP (Owner session): `GET /api/v1/outbound/policy`; `GET/POST /api/v1/smtp/outbounds`;
`POST /api/v1/smtp/outbounds/test` (unsaved settings, optional `id` to reuse a stored password);
`GET/PUT/DELETE /api/v1/smtp/outbounds/<id>`; `PUT /api/v1/smtp/outbounds/<id>/enabled`;
`POST /api/v1/smtp/outbounds/<id>/test`; `GET/POST /api/v1/scopes/<id>/routing-actions`;
`PUT/DELETE /api/v1/routing-actions/<id>`; `GET /api/v1/outbound/deliveries?status&before&limit`;
`GET /api/v1/outbound/deliveries/<id>` (with attempts); `POST /api/v1/outbound/deliveries/<id>/retry`;
`GET /api/v1/messages/<id>/deliveries`. Timestamps are Unix seconds. Electron uses the
`sandpost:admin:outbound:*`, `sandpost:admin:routing:*` and `sandpost:admin:deliveries:*` channels
with the same bodies.

The Vue interface adds **Administration → SMTP Outbound** (list with enable switches, create/edit
dialog with basic and advanced settings and recipient allowlist, connection tests, confirmed
deletion, and a compact recent-deliveries list with history and retry), a **Routing** section in the
scope editor (add relay/forward, choose an existing outbound, edit, enable, delete), and, for Owners
only, a **Delivery** tab in the message viewer that appears only when the message has deliveries.

## Storage

Migrations 0054–0059 create `smtp_outbounds`, `scope_routing_actions` (cascading from `scopes`,
restricting `smtp_outbounds`), `mail_routing_evaluations` (cascading from `mail`),
`outbound_deliveries` (cascading from `mail`; scope, action and outbound references become null when
those are deleted), `outbound_delivery_recipients` and `outbound_delivery_attempts` (cascading from
their delivery). See [storage.md](storage.md).

## Limitations

- Only `AUTH PLAIN` and `AUTH LOGIN` are supported; no OAuth2/XOAUTH2, client certificates, DSN,
  SMTPUTF8 envelopes, REQUIRETLS or DKIM signing.
- The `SIZE` limit is not checked in advance; an oversized message fails with the server's 5xx.
- Retries and backoff are per delivery job; there is no per-domain throttling or MX lookup. An SMTP
  Outbound always targets its configured host (a smarthost), never recipients' MX servers.
- Routing actions are explicit per scope; there is no inheritance and no routing for mail captured
  before the action existed.
