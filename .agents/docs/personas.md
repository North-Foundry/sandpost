# Personas and PersonaScript

A **Persona** is a user-owned virtual recipient. When captured mail addressed to it can be
read by its owner, Sandpost runs the Persona's **PersonaScript**: it may wait, open the message
(loading its remote images), evaluate Sandpost query conditions, and click links. Everything runs
on the server, headless, in the CLI, Docker image and desktop runtime alike.

## Ownership and management

- Every Persona has exactly one owning user (`personas.owner_identifier`, cascading with the
  user). Personas are never owned by scopes, SMTP accesses or IMAP accounts and never grant or
  change access.
- The address is normalized (trimmed, ASCII-lowercased, same dot-atom policy as address inboxes)
  and **unique across the installation**. SQLite enforces it with `UNIQUE` plus a `CHECK` that the
  stored value is already normalized; creation and updates also check inside an immediate
  transaction, so concurrent attempts cannot both succeed (`StorageError::DuplicatePersonaAddress`,
  HTTP 409 "this email address is already used by a persona"). The conflict reveals only that the
  address is taken, which global uniqueness cannot avoid; it never names the Persona or its owner.
  Names need not be unique.
- Owners and Members create, edit, enable, disable and delete their own Personas (at most 50
  each). Viewers cannot create or change Personas. A user who cannot see a Persona receives 404
  for every Persona route, so identifiers cannot be probed.
- Global **Owners** may additionally list every Persona (`GET /api/v1/personas?owner=all`), read,
  enable, disable and delete any Persona. They cannot rewrite another user's name, address or
  script (403): administration stops misbehaving Personas without impersonating their owner.
- Deleting a Persona deletes its execution history; messages it received are never deleted.
- Each Persona may store an optional `icon`, using the same lowercase kebab-case Lucide name
  contract as folders (at most 64 ASCII characters). Missing or null values select the default
  mask; unknown well-formed names are kept and displayed with the default icon. An icon-only
  edit preserves the execution revision and does not interrupt running or resumed scripts.

## Address check at creation

The owner's composed authorization (mail access, assigned scopes with inherited filters, and
personal filter) is evaluated against the address with three-valued logic, because scopes are
arbitrary queries rather than domain rules. Only envelope-recipient predicates can be decided:
every message addressed to the Persona lists its address among its SMTP envelope recipients, so a
predicate it satisfies is certainly true (collection comparisons use ANY), while an unsatisfied one
stays unknown because another recipient may satisfy it. Other fields are unknown. Single predicates
are evaluated by `sandpost-query` itself.

| Result | Meaning | Effect |
| --- | --- | --- |
| `confirmed` | Every message to the address is readable | Created |
| `per_message` | Access depends on each message | Created; authorization is enforced per execution |
| `denied` | No message to the address can be readable (for example `Scoped` without assignments) | Refused with 400 |

`address_access` is recomputed on every read, so it follows later scope changes. It is advisory:
execution never relies on it.

## Matching and scheduling

Matching uses the **SMTP envelope recipients** (`RCPT TO`, which includes Bcc). Every captured
message has them, and they are what an address inbox filters on; MIME `To`/`Cc` headers can be
forged or omitted and are ignored. Comparison ignores ASCII case. A Persona appearing several times
in one envelope is scheduled once.

`MessageStorage::insert_message` schedules, inside the same transaction that stores the message,
one `pending` execution per **enabled** Persona whose address matches, unique per Persona and
message. A Persona already holding 1000 pending executions receives nothing more, so a burst cannot
queue unbounded outbound work. SMTP acknowledges after this commit as before; no script runs on
the ingestion path. Scheduling authorizes nothing.

## Authorization during execution

Before the first step, before every `open` and `click`, and after every `wait`, the runtime
re-reads current state and requires, in this order: the Persona exists; its owner exists; the owner
can read the message now (the owner's own deletions plus the same composed authorization as direct
reads, evaluated against the current SQL facts); the owner is not a Viewer; the Persona is enabled;
and the message's envelope still includes the Persona's current address.

- If the owner could not read the message when the execution was first considered, the execution
  is **discarded** without trace, so history never reveals inaccessible mail.
- Losing access, disabling, changing the address, deleting the Persona, or demoting the owner to
  Viewer later **cancels** the execution with a reason before its next action.
- Gaining access never replays old mail: executions exist only for mail stored while the Persona
  was enabled, and an execution discarded for missing access is gone.
- History and the Persona mail view are filtered by the **reader's** current authorization and
  deletions, also for Owners administering another user's Persona.

## Execution lifecycle

Statuses: `pending`, `running`, `completed`, `failed`, `cancelled`. Each step (`branch`, `open`,
`wait`, `click`) is journaled in `persona_execution_actions` as `started` **before** its effect and
updated to `succeeded`, `failed`, `interrupted` or `cancelled`, with line/column, start time,
duration, error and a typed JSON detail (image requests, selected link, request outcome, condition
outcome, wait length).

- The dispatcher (`Application::run_personas`) wakes on message notifications, Persona changes,
  finished executions and a two-second poll. It starts interrupted (`running`) executions first,
  then `pending` ones, oldest first, while fewer than `concurrent_executions` are active. Each
  execution is an independent Tokio task outside transport supervision: a failing, slow or
  panicking execution affects no other execution, listener or user.
- Waits are `tokio::time::sleep`, so waiting executions hold no thread and no request permit.
- An execution's time limit (`execution_timeout`, default 30 minutes) is measured from its first
  start, also across restarts. Scripts whose longest path of waits exceeds three quarters of it are
  rejected when saved.
- A script edit applies to executions that start afterwards; a running execution keeps the
  revision it started with. Disabling or changing the address cancels pending and running ones.
- Finishing an execution keeps the newest 500 terminal executions per Persona.

### Restart recovery and idempotency

Outbound requests are **never retried**: a click or tracking pixel may not be idempotent, so a
request that may have reached its server is reported once. Shutdown stops executions without
finishing them. On the next start an interrupted execution replays the same deterministic program
(conditions evaluate the immutable message facts) against its journal: finished steps are skipped,
an interrupted `wait` sleeps only its remaining time (second precision), and an interrupted `open`
or `click` is marked `interrupted` and fails the execution without being repeated. A changed script
revision fails the resumed execution. A storage error pauses an execution, which a later poll
resumes from its journal under the same rules.

## PersonaScript

`crates/sandpost-persona-script` has a dedicated lexer, a recursive-descent parser producing a
typed syntax tree, a lowering to a flat forward-jump program, and an interpreter that delegates
effects to an `ActionHandler`. It has no storage, HTTP server, SMTP or frontend dependency.

```text
script        = { statement }
statement     = "open" ";" | "wait" DURATION ";" | "click" link_selector ";" | if_statement
if_statement  = "if" "(" CONDITION ")" block [ "else" ( if_statement | block ) ]
block         = "{" { statement } "}"
link_selector = "link" "(" ( STRING | ( "text" | "href" ) ":" STRING ) ")"
DURATION      = DIGITS ( "ms" | "s" | "m" )
```

- Keywords are case-sensitive. Strings use `"` or `'` with the query language's escapes. `//` and
  `/* */` are comments. Empty statements and stray braces are errors.
- `CONDITION` is the Sandpost query language. The parser hands the text after `if (` to
  `sandpost_query::compile_embedded`, which tokenizes with the query lexer up to the parenthesis
  that closes the condition, parses with the query grammar and canonicalizes; evaluation uses
  `CompiledQuery::evaluate`. Every query field and operator is available with identical semantics
  and diagnostics; an empty condition is an error.
- `open;` loads the remote images a mail client would: `<img src>` (or the first `srcset`
  candidate when `src` is absent), `<input type="image" src>` and `background` attributes, at most
  32 distinct URLs per open, four at a time. `cid:` and `data:` images are counted as embedded.
  Stylesheets and CSS `url()` are not loaded and no script runs. Failed images do not fail `open`.
  Opening never changes the owner's read state.
- `click link("css")` takes the first matched element that is an `<a href>`/`<area href>`, or
  else the closest link enclosing a matched element (clicking it would follow that link).
  `link(text: "…")` matches the visible text (descendant text and image `alt`; an area's `alt`)
  after collapsing whitespace, as a whole-value glob ignoring ASCII case. `link(href: "…")`
  matches the resolved absolute URL as a case-sensitive whole-value glob. Globs are the query
  language's `matches` (`sandpost_query::glob_matches`): `*` any run, `?` one character. Selecting
  a link that cannot be followed is an error rather than a fall-through to another link. No match
  fails the execution with a message such as "none of the message's 3 link(s) has matching text".
  Any HTTP response completes the click; no response fails it.
- URL policy: references are resolved against the document's first absolute `http(s)` `<base
  href>`; without one, protocol-relative references use `https` and other relative references are
  refused. Only `http`/`https` URLs with a host are requested; `javascript:`, `mailto:`, `data:`,
  `file:` and other schemes are refused, as are URLs containing a user name or password. Fragments
  are dropped. HTML is parsed with html5ever, so malformed markup is repaired like a browser would.
- Diagnostics report the first problem with message, UTF-8 byte position, 1-based line and
  character column. Unknown instructions, invalid durations or units, invalid CSS selectors,
  unknown or missing `link` arguments, malformed blocks and invalid conditions are rejected before
  a script is stored. Limits: 16 KiB source, 256 statements, nesting depth 16, single `wait` at
  most `maximum_wait` (default 10 minutes), longest wait path at most three quarters of the
  execution timeout. Scripts are compiled when saved and the program is cached per Persona
  revision, so incoming mail never reparses source.
- There are deliberately no variables or loops. The flat program uses forward jumps only, so
  future bounded loops can be added as new statement kinds and instructions without redesign.

## Network security

Requests use `sandpost-link-check`'s client through `LinkChecker::visit`: one `GET` per hop,
redirects followed manually (at most 10, loops and non-HTTP targets fail), and the final body read
up to `maximum_response_bytes` (1 MiB) before the connection is closed. Every hop's host name is
resolved once and the connection uses only those addresses; if any resolved address is loopback,
private, carrier-grade NAT, link-local (including `169.254.169.254` cloud metadata), multicast,
documentation, benchmarking, reserved, unspecified, or an IPv4-mapped/NAT64/6to4 form of one, the
request is refused without connecting, including after redirects and for mixed or rebinding DNS
answers. Timeouts: 5 s to resolve, connect or handshake TLS; 10 s for response headers; 20 s per
request including redirects and body. At most `concurrent_requests` (8) requests run instance-wide.
Requests send a `Sandpost-Persona` user agent, `Accept`, `Accept-Encoding: identity` and the
`Host`, never cookies, credentials, referrers or Sandpost tokens. HTTPS verifies certificates with
the system trust store.

`[personas] allow_internal_addresses = true` (or
`SANDPOST_PERSONAS_ALLOW_INTERNAL_ADDRESSES=true`) permits non-public destinations for trusted
single-developer machines testing local applications; leave it off on shared instances.

### Limitations of HTTP-based simulation

Personas are not browsers: no JavaScript, no CSS layout, no stylesheet or font loading, no cookie
jar between requests, no form submission, no client fingerprint, and links are followed with plain
`GET` requests whose landing pages' subresources are not loaded. Image proxies used by real mail
clients (such as Gmail's) are not imitated. Tracking pixels, click tracking, redirect chains,
verification and unsubscribe links that work through `GET` are exercised faithfully.

## Configuration

| TOML `[personas]` | Environment | Default | Meaning |
| --- | --- | --- | --- |
| `allow_internal_addresses` | `SANDPOST_PERSONAS_ALLOW_INTERNAL_ADDRESSES` | `false` | Permit non-public destinations |
| `maximum_wait_seconds` | `SANDPOST_PERSONAS_MAXIMUM_WAIT_SECONDS` | `600` | Longest single `wait`; at most ¾ of the timeout |
| `execution_timeout_seconds` | `SANDPOST_PERSONAS_EXECUTION_TIMEOUT_SECONDS` | `1800` | Whole execution, 4–86400 |
| `concurrent_executions` | `SANDPOST_PERSONAS_CONCURRENT_EXECUTIONS` | `64` | Active executions, waiting included, 1–1024 |
| `concurrent_requests` | `SANDPOST_PERSONAS_CONCURRENT_REQUESTS` | `8` | Outbound requests in flight, 1–256 |

## Interfaces

HTTP (session required; ownership rules above): `GET/POST /api/v1/personas` (`?owner=all` for
Owners), `POST /api/v1/personas/validate` `{ script }`, `GET/PUT/DELETE /api/v1/personas/<id>`
with `{ name, address, icon?, script, enabled? }`, `PUT /api/v1/personas/<id>/enabled` `{ enabled }`,
`GET /api/v1/personas/<id>/messages?before&limit&q`, `GET /api/v1/personas/<id>/executions?before&limit`
and `GET /api/v1/personas/<id>/executions/<execution>`. Timestamps are Unix seconds; step
durations are milliseconds. Electron exposes the same operations through
`sandpost:personas:list/get/create/update/enabled/delete/validate/messages/executions/execution`.

The Vue sidebar shows a **Personas** section below Mail. Each Persona is a filtered mail view of
readable mail whose envelope includes its address (filed mail included, Trash and deleted mail
excluded, like Smart Folders), using the ordinary list and viewer. Its header shows the address,
disabled state and an access hint, with **Activity** (a side sheet of runs and their steps,
refreshed while runs are unfinished) and **Edit**. The editor is CodeMirror with PersonaScript
highlighting (query highlighting inside conditions), line numbers, statement completion and server
validation underlining the reported position. Creation and editing reuse the searchable folder
icon picker, including reset to the default mask. The chosen icon is persisted and used in the
sidebar; a disabled Persona keeps the pause indicator until re-enabled.

## Storage

Migrations 0051–0053 create `personas`, `persona_executions` (unique Persona/message, cascading
from Persona and from `mail`) and `persona_execution_actions` (keyed by execution and step,
cascading from the execution, JSON detail checked with `json_valid`). See [storage.md](storage.md).
