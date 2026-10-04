# Sand Post

A self-hosted development email catcher in Rust. This repository establishes the
domain, query and matching foundations; it does not yet implement the complete
multi-user product.

## Build and run

Use Rust 1.89 or newer with Cargo and a C compiler for bundled SQLite. The
current lockfile was verified with Rust 1.98. No database server, queue or
cache service is required.

```sh
cargo build --workspace --locked
cargo run -p sandpost --locked
```

Read captured messages at <http://127.0.0.1:8025/api/v1/messages>.
Send mail to `127.0.0.1:1025`:

```sh
python3 - <<'PY'
import smtplib
from email.message import EmailMessage
m = EmailMessage()
m['From'] = 'dev@bflow.dev'
m['To'] = 'developer@boris.it'
m['Subject'] = 'Hello Sand Post'
m.set_content('Captured locally.')
with smtplib.SMTP('127.0.0.1', 1025) as smtp:
    smtp.send_message(m)
PY
```

After storage and both listeners initialize, startup prints a plain-text summary
to stdout. Operational tracing goes to stderr; ANSI colors are used only for a
TTY when `NO_COLOR` is absent. The summary reports the actual
`GET /api/v1/messages` endpoint, listener bind addresses, SMTP host and actual
port, SQLite path, and that SMTP encryption and authentication are disabled
(credentials are not required). Attachments remain inside each original message
in SQLite. For client connection addresses, wildcard binds map to the matching
address-family loopback (`127.0.0.1` or `::1`); the `Listening` row still shows
the actual bind address. If configured with port `0`, the summary shows the
system-allocated port. The following is the current-default excerpt, omitting
the title, subtitle, and final readiness line:

```text
SMTP
  Host       127.0.0.1
  Port       1025
  Encryption none
  Auth       disabled (no credentials required)
  Listening  127.0.0.1:1025

HTTP
  API        http://127.0.0.1:8025/api/v1/messages
  Listening  127.0.0.1:8025

Storage
  Database     SQLite · data/sandpost.sqlite3
  Attachments  in original messages (SQLite)
```

The binary bundles SQLite and serves the HTTP API without a browser interface.
Mail is stored once in `data/sandpost.sqlite3`; extracted facts and attachment metadata use relational
columns and child rows, while the original raw message retains attachment
bytes. Public Rust and HTTP representations keep their existing field names.
Startup creates a stable **All Mail** scope. SMTP only
acknowledges a message after its storage transaction commits. SIGINT/SIGTERM stop
the listeners with bounded draining.

Generate repeating SMTP traffic with Python's standard library:

```sh
python3 scripts/mail_bursts.py
# Stop after one cycle (106 emails):
python3 scripts/mail_bursts.py --cycles 1
```

Each cycle sends 100 emails as fast as SMTP accepts them, pauses for 30 seconds,
then sends 6 emails at 5-second intervals over 30 seconds before repeating.
Stop with Ctrl+C. The defaults are `127.0.0.1:1025`; override with `--host` and
`--port`.

**Current operating mode:** unauthenticated local development. HTTP lists all
captured mail and SMTP accepts local submissions; keep these listeners on
loopback. The authorization primitives are tested library code, not login or
API middleware. Binding externally does not enable authentication.

## Configuration

| Environment variable | Default | Meaning |
| --- | --- | --- |
| `SANDPOST_HTTP_LISTEN` | `127.0.0.1:8025` | HTTP socket address |
| `SANDPOST_SMTP_LISTEN` | `127.0.0.1:1025` | SMTP socket address |
| `SANDPOST_DATA_DIR` | `data` | Local data directory |
| `SANDPOST_DATABASE_PATH` | `<data directory>/sandpost.sqlite3` | SQLite file |
| `SANDPOST_MAX_SCOPE_DEPTH` | unset, unlimited | Maximum depth; roots are depth zero |
| `SANDPOST_LOG_LEVEL` | `info` | Tracing filter, e.g. `info,sandpost_match=debug` |

Malformed configuration, invalid persisted topology/filter, failed migrations
and listener bind errors stop startup. SMTP bounds message size to 10 MiB,
recipient count to 100, concurrent sessions to 32 and idle operations to 60s.
This is a catcher, with no outbound delivery, TLS, SMTP AUTH or enterprise SMTP
features.

## HTTP API

| Method and path | Result |
| --- | --- |
| `GET /api/v1/health` | Readiness including a database probe |
| `GET /api/v1/messages?limit=50&before=<seq>` | Newest first, keyset pagination, max 100 |
| `GET /api/v1/messages/<uuid>` | Normalized facts and attachment metadata |
| `GET /api/v1/messages/<uuid>/raw` | Original captured `.eml` |
| `GET /api/v1/events` | SSE `ready`, `message`, `resync` events |

SSE is a bounded notification stream; it has no durable replay. Reload the
messages API on `ready` (including reconnect) or `resync`. Events contain public
message IDs and internal pagination sequences, not message bodies.

## Verify

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
python3 scripts/smoke.py
```

The smoke test starts the actual binary using isolated data/ports, sends SMTP
mail, verifies MIME facts, materialized scope links, HTTP, raw data and SSE, then
restarts the process and verifies persistence and null SMTP senders.

## Architecture and next steps

- [System architecture and component boundaries](architecture.md)
- [Persistence contracts and SQLite storage](storage.md)
- [Typed query language and semantics](query-language.md)
- [Code style and complete naming rules](code-style.md)
- [Scope engine, candidate selection and policy updates](scope-engine.md)

Implemented: generic scope hierarchy, many-scope membership, typed DSL,
canonicalization, shared expression nodes, exact candidate indexes, per-message
memoization, inherited restrictions, set deltas, SQLite migrations/materialized
matches, MIME normalization, bounded SMTP, versioned HTTP/SSE.

Next milestones, in order:

1. Authentication, sessions and API/SSE authorization using current materialized
   scope grants plus personal restrictions.
2. Transactional scope editing and policy publication, with targeted historical
   reclassification and resumable delta jobs.
3. Retention and deletion, disk attachment extraction, bounded storage scheduling
   and representative matching/ingest benchmarks.
4. Rich message inspection and saved inboxes.

Intentionally deferred: passwords/sessions, scope CRUD endpoints, full historical
recomputation, compressed bitmaps, FTS, SMTP AUTH/TLS, disk attachment blobs,
durable SSE replay and the complete UI. No fabricated implementation stands in
for these features.
