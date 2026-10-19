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

Requirements: Rust 1.99+, Cargo, a C compiler (bundled SQLite), Node.js 24 LTS, and Yarn 4. Use `nvm use` to select the version declared in `.nvmrc`. Install JavaScript dependencies at the repository root; Rust crates share the root workspace and lockfile.

```sh
yarn install
cargo build --workspace --locked
cargo run -p sandpost --locked
```

Defaults are HTTP `127.0.0.1:8025`, SMTP `127.0.0.1:1025`, and IMAP `127.0.0.1:1143`. `sandpost-web-api` embeds `apps/frontend/dist` at compile time when it exists, so a build after `yarn workspace @sandpost/frontend build` serves the web interface from the single binary and a frontend change recompiles it. Without a configured frontend proxy, HTTP serves embedded assets when present and otherwise only the API. The runtime serves embedded assets and uses `index.html` for non-API SPA paths; unknown `/api` paths remain 404. Node is a build dependency only; the runtime needs no external frontend directory.

`yarn run sandpost-cli:build` builds the CLI and its frontend prerequisite. `yarn run sandpost-cli:dev` asks Turbo to run the ordinary `dev` task concurrently for Sandpost and its dependency graph. Each package has only one `dev` task. The shared `@sandpost/vite-plugin-hot-file` workspace in `packages/vite-plugin-hot-file` exports `hotFile()` directly from TypeScript source, which any Vite configuration can include. Node.js 24 runs the plugin tests directly from TypeScript; `check` performs separate type checking. It finds the workspace root and atomically writes the actual HTTP origin as plain text to `.vite-hot-file` there once listening, removing its publication on normal shutdown. The frontend configuration uses this plugin for browser, mock and current desktop starts; future Vite configurations can import the same package. The file is ignored by Git and shared by frontend dev servers; the latest publication wins. The crate's `dev` command runs Cargo with `--frontend-proxy-file ../../.vite-hot-file`. Rust reads that file directly, waiting up to 30 seconds for publication. No process output is parsed. The CLI listens at `http://127.0.0.1:8025` and proxies non-API HTTP requests and WebSockets to that Vite server, providing the UI and HMR at the CLI URL; the announced Vite URL remains available for opening the frontend directly. Stopping the command stops both frontend and backend; Turbo cancels the other task if one fails. The proxy URL is reported as `Frontend proxy` even when no embedded `dist` exists. The proxy can be configured in debug and release builds; if its upstream is unavailable, proxied requests return 502. The Rust CLI binary is built for the host platform at `target/release/sandpost` (Windows: `sandpost.exe`). Use `yarn workspace @sandpost/frontend dev:mock` for Vite with the explicit mock client, `yarn workspace @sandpost/frontend build` to type-check and build, and `yarn workspace @sandpost/frontend test` for frontend tests. `yarn workspace @sandpost/desktop dev` builds the native addon and Electron main/preload, runs Vite on 1420, and launches Electron; browser Vite uses 5173. The desktop workspace's `native:build`, `check`, `test`, `smoke`, `smoke:preview`, `build:dir`, and `build` scripts cover addon, IPC, Electron and packaging tasks. Run `node scripts/native-smoke.cjs` for native runtime smoke coverage after building the addon. Pass `--dev` to desktop smoke for the development app. Electron ships Vue assets and native artifacts outside ASAR; packaged users need no toolchain. The CLI release workflow builds macOS ARM64/x64 and Ubuntu 26.04 GNU x86_64 binaries. Windows x86_64 builds are temporarily disabled while real-process frontend proxy startup tests are repaired. Ubuntu 26.04 is the Linux build and smoke-test baseline, pinned with the `ubuntu-26.04` runner label. GNU binaries may require the build image's glibc version, so compatibility with older Ubuntu versions, Alpine and other Linux distributions is not promised. macOS builds are unsigned and not notarized. Until release CI has run successfully, only macOS ARM64 has local native/release build verification; the other targets are configured, not validated.

From any workspace, run `yarn run sandpost-cli:dev` and open `http://127.0.0.1:8025` to use the live frontend. To run the backend separately against an existing Vite server, use `cargo run --locked -p sandpost -- --frontend-proxy-url http://127.0.0.1:5173`. The crate-local `yarn run dev` starts only Rust and reads the file from an already running frontend. The `--frontend-proxy-file PATH` option exists only when Rust debug assertions are enabled; it is rejected and its reader is not compiled in release builds. `--frontend-proxy-url URL` exists in every profile and takes precedence when both flags are supplied, so the file is neither read nor awaited. Without the explicit URL, the debug file value overrides TOML. Neither profile discovers a proxy file implicitly; the crate script supplies its path. The optional TOML setting `[frontend] proxy_url = "http://frontend:8080"` configures the same proxy; the CLI flag overrides it. The URL must be an HTTP or HTTPS origin with a DNS name or IP address and root path only, without credentials, query, or fragment. With a proxy configured, `/api` requests stay on Rust and other HTTP requests and WebSockets go to the upstream. Without a proxy setting, embedded assets are served when available; otherwise HTTP serves only the API. WebSocket tunnels close when the runtime shuts down.

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

After storage/listener initialization, stdout prints a plain-text connection summary; tracing goes to stderr, colors require a TTY and no `NO_COLOR`. It reports the Web UI URL when the frontend is embedded, the `Frontend proxy` URL when configured, actual API/bind addresses, SMTP host/port and encryption/AUTH state, SQLite path, the configuration file when one was read, and IMAP connection/security or disabled status. Wildcard binds map client instructions to the matching loopback family while `Listening` shows the actual bind; port 0 reports the allocated port. An SMTP bind failure stops startup. The SMTP listener is configured only through environment variables, TOML, and defaults; its address, status, and security settings are read-only in the interface and require a restart to change.

When no Owner exists, each startup prints a fresh 256-bit setup key and local `/setup` URL. `GET /api/v1/instance` reports `{ initialized, setupKeyRequired }`; the browser verifies the key at `POST /api/v1/setup/authorize`, then submits Owner fields with the temporary authorization in `bootstrapSecret` to `POST /api/v1/setup`. Authorizations expire in 10 minutes, at most 64 are kept (oldest evicted), and restart or successful setup invalidates them. The legacy field also accepts the boot key. Setup creates no session; login follows. The key is never exposed by HTTP or persisted in frontend storage. Setup remains available whenever storage has no Owner, even if non-Owner accounts or `instance.json` exist; Owner creation is atomic, preserves those accounts, and honors unique email. Metadata is written after Owner creation; a metadata-write failure locks setup for that process, while restart recognizes the committed Owner.

Setup validation/server errors appear inline. If another client initializes the instance, setup exits to login or the authenticated app; refresh rereads status without persisting the key or authorization.

Electron is trusted: native status says no setup key is required, and `NativeRuntime::setup` injects the boot key through origin-checked IPC; the renderer supplies only Owner fields. The mock uses `sandpost-test-secret`, the same authorization flow, and a 12-byte minimum password.

### Container image

The root `Dockerfile` builds the Vue frontend with Node.js 24 and the Rust CLI
with the musl target, then copies the static executable into `scratch`. Node,
Rust, package managers and a shell are not included in the runtime image. The
runtime includes a CA trust bundle for HTTPS frontend proxy connections and runs
as UID/GID `65532:65532`. The image defaults to HTTP/SMTP on `0.0.0.0`, stores
data in `/data`, and disables IMAP until explicitly configured. SMTP retains its
ordinary AUTH policy, requiring credentials on the wildcard bind.

Build from the repository root:

```sh
docker build -t sandpost:local .
```

The `verification` build target compiles the existing smoke runner and checks the
musl executable, embedded frontend, SMTP/HTTP/IMAP, indexing and restart behavior
inside the builder. The runner and its dependencies do not enter the runtime image:

```sh
docker build --target verification .
```

Mount all of `/data` persistently, including the derived `search/` directory.
Keeping only SQLite forces an index rebuild before listeners open on the next
startup. The image prepares `/data` for its runtime user; Docker named volumes
inherit this ownership when initially populated. Host bind mounts must already
be writable by UID/GID `65532:65532`. The runtime supports a read-only root
filesystem with `/data` writable.

Run the local image, or replace `sandpost:local` with a published version:

```sh
docker run --name sandpost --rm \
  -p 127.0.0.1:8025:8025 -p 127.0.0.1:1025:1025 \
  --mount type=volume,source=sandpost-data,target=/data \
  --read-only sandpost:local
```

Read the key using `docker logs sandpost`. The volume preserves SQLite, the search index and `instance.json`. Mounting a TOML file and pointing `SANDPOST_CONFIG` at it configures the process from that file; environment values override TOML. A non-loopback SMTP bind requires AUTH by default; create an SMTP access or explicitly set `SANDPOST_SMTP_AUTH=optional`. The example disables IMAP. Non-loopback IMAP needs a certificate (`SANDPOST_IMAP_TLS_CERTIFICATE`/`_PRIVATE_KEY`, or the SMTP pair) and a published port, e.g. `-p 127.0.0.1:993:993 -e SANDPOST_IMAP_ENABLED=true -e SANDPOST_IMAP_BIND_ADDRESS=0.0.0.0 -e SANDPOST_IMAP_PORT=993`; without a certificate startup refuses it unless `SANDPOST_IMAP_TLS=off`, which permits clear-text passwords. Image publication is configured in [versioning.md](versioning.md#container-publication).

`sandpost-test-runner mail-burst` sends synthetic traffic to an already running SMTP server, defaulting to `127.0.0.1:1025`. It shares the runner package with `smoke-test`, which starts its own isolated server. Run through Cargo as follows:

```sh
# Send 50,000 messages consecutively, then exit.
cargo run --locked -p sandpost-test-runner -- mail-burst full --count 50k

# Send four groups, with ten seconds between them; one cycle by default.
cargo run --locked -p sandpost-test-runner -- mail-burst tiny --pattern '50,10,1,50' --interval 10s

# Custom pauses and a freshly sampled inclusive random count, repeated twenty times.
cargo run --locked -p sandpost-test-runner -- mail-burst tiny --pattern '50,10s,10,1s,rand(10,100)' --cycles 20
```

`full` requires a positive `--count` and has no artificial pauses. `tiny` requires `--pattern`; counts are positive integers with optional decimal `k`/`K` suffixes, and `rand(min,max)` samples a new inclusive integer each time that group runs. An explicit pause after a quantity overrides `--interval` for that boundary. Durations are nonnegative integers with `ms`, `s`, or `m` units. The default interval is one second; it also separates the final group of one cycle from the first group of the next. No pause follows the final finite group, including an explicitly written trailing pause. `--cycles` defaults to one; zero repeats until Ctrl+C. Quote patterns so the shell does not interpret parentheses. Malformed patterns, incompatible options and finite count overflows fail before sending.

Both modes accept `--host`, `--port`, `--username`, and `--password-env`. AUTH LOGIN reads its password from `SANDPOST_SMTP_PASSWORD` by default, keeping it out of command arguments and logs. One SMTP connection is reused within each group and closed before pauses; the generator neither adds TLS nor automatically retries a failed message. Messages have unique IDs, dates and cycle/group/message metadata. Only final successful SMTP DATA replies count as accepted; a lost reply can leave delivery unconfirmed and is never retried. Terminal output reports cycle/group progress, accepted counts, elapsed time, average throughput and pauses. Ctrl+C stops between operations or interrupts a pause promptly, prints the confirmed totals and exits 130; an in-flight network operation is bounded by its five-second socket timeout. SMTP failures print partial totals and exit unsuccessfully.

### Security and native preview

Electron enables renderer context isolation/sandboxing and disables Node integration. IPC validates the owned window, main frame and trusted origin; preload exposes no filesystem or shell access.

HTTP has no built-in TLS. Without SMTP/IMAP certificates, credentials and HTTP sessions are clear text; keep listeners on loopback or protect them with a trusted boundary/TLS-terminating proxy. SMTP AUTH defaults to optional on loopback and required elsewhere; TLS `auto` requires STARTTLS on non-loopback when a certificate exists. IMAP clear text is loopback-only unless explicitly opted out of TLS. See Configuration and [imap.md](imap.md) for exact protocol and mailbox behavior.

Browser HTML preview uses an empty-sandbox iframe and restrictive content policy. Electron uses a dedicated `WebContentsView` with no preload, Node access, application session, or persistent storage; a private session serves only the current document through `sandpost-email:`. Response CSP includes an empty `sandbox`; scripts, remote/application resources, nested frames, connections, permissions, downloads, popups, and navigation are blocked, while inline CSS and data images work. The same wrapped document is used in both previews; no separate HTML sanitizer is claimed.

The desktop-only Inspect control attaches a fresh DevTools `WebContentsView` to the email target inside the existing window (no extra window and never the app renderer). It starts closed at about 35% width, with pointer/keyboard divider resizing. The email target survives message changes; generation checks discard stale loads. Chromium's bundled settings place the first Elements/Styles sidebar on the right, preserve later window-session settings, and apply the resolved light/dark theme, including system appearance. The inspector stays hidden until settings/theme are ready and its first themed frame is painted; Vue shows a themed animated loader until readiness and a scroll hint only when a ready inspector is out of view. Reduced-motion is honored.

Vue sends viewport rectangles over `sandpost:preview:*` IPC, validating the owned application main frame and payloads; main converts them using renderer zoom and updates native bounds on scroll/resize/layout. Native views hide while dragging, overlays are open, or either region is clipped; the iframe remains the clipped fallback. The inspector needs both regions fully visible and is bounded by the scrollable viewer/minimum window. Failures fall back to browser preview. Both views are destroyed on unmount or host navigation/crash/close. In pinned Electron 44.5.1, the caller-owned DevTools frontend survives `closeDevTools()`/`close()`, so cleanup calls the native `destroy()` binding (missing from TS declarations); smoke coverage checks cleanup/reopen. Recheck Chromium settings and this binding on Electron upgrades.

## Configuration

| Environment variable | Default | Meaning |
| --- | --- | --- |
| `SANDPOST_CONFIG` | `~/.sandpost/config.toml` | Configuration file read at startup; an explicitly named file must exist |
| `SANDPOST_HTTP_BIND_ADDRESS` | `127.0.0.1` | HTTP interface address |
| `SANDPOST_HTTP_PORT` | `8025` | HTTP port; `0` asks the operating system to allocate a port |
| `SANDPOST_SMTP_BIND_ADDRESS` | `127.0.0.1` | SMTP interface address |
| `SANDPOST_SMTP_PORT` | `1025` | SMTP port; `0` asks the operating system to allocate a port |
| `SANDPOST_SMTP_AUTH` | `auto` | SMTP AUTH requirement: `auto` (optional on loopback, required elsewhere), `required`, or `optional` (explicit opt-in to unauthenticated mail on any address) |
| `SANDPOST_SMTP_TLS` | `auto` | SMTP TLS requirement: `auto` (with a certificate, required on non-loopback addresses and offered on loopback; without one, no TLS), `required` (needs a certificate), or `off` |
| `SANDPOST_SMTP_TLS_MODE` | `starttls` | `starttls` (clients upgrade with STARTTLS, ports 25/587) or `implicit` (TLS from the first byte, port 465; needs a certificate) |
| `SANDPOST_SMTP_TLS_CERTIFICATE` | unset | PEM certificate chain for SMTP TLS; overrides `smtp.tls_certificate`. Pair with the resolved private key |
| `SANDPOST_SMTP_TLS_PRIVATE_KEY` | unset | PEM private key for SMTP TLS; overrides `smtp.tls_private_key`. Pair with the certificate; an unreadable or mismatched pair stops startup |
| `SANDPOST_IMAP_ENABLED` | `true` | Enable the IMAP listener; set `false` to disable it while retaining all IMAP accounts and data |
| `SANDPOST_IMAP_BIND_ADDRESS` | `127.0.0.1` | IMAP interface address |
| `SANDPOST_IMAP_PORT` | `1143` | IMAP port; `0` asks the operating system to allocate a port |
| `SANDPOST_IMAP_TLS` | `auto` | IMAP TLS requirement: `auto` (with a certificate, TLS; STARTTLS mode requires it on non-loopback addresses; without one, clear text allowed only on loopback), `required` (needs a certificate), or `off` (explicit opt-in to clear-text passwords on any address) |
| `SANDPOST_IMAP_TLS_MODE` | `implicit` | `implicit` (IMAPS, port 993) or `starttls` (port 143) |
| `SANDPOST_IMAP_TLS_CERTIFICATE`, `SANDPOST_IMAP_TLS_PRIVATE_KEY` | the SMTP pair | PEM certificate chain and key for IMAP; set both or neither. An unreadable or mismatched pair stops startup |
| `SANDPOST_IMAP_HOST` | bound address | Advertised host shown in IMAP connection instructions; blank after trimming uses the bound address |
| `SANDPOST_IMAP_MAX_CONNECTIONS` | `64` | Concurrent IMAP sessions (1 to 10000) |
| `SANDPOST_SMTP_ENDPOINT`, `SANDPOST_SMTP_ENDPOINTS` | removed | SMTP endpoints no longer exist; setting either fails startup with a migration hint |
| `SANDPOST_DATA_DIRECTORY` | platform data directory | Local data directory; overrides `storage.data_directory` in the configuration file; blank after trimming is treated as unset |
| `SANDPOST_DATABASE_PATH` | `<data directory>/sandpost.sqlite3` | SQLite file; overrides `storage.database_path`; blank after trimming is treated as unset |
| `SANDPOST_MAX_SCOPE_DEPTH` | `2` | Overrides `sandpost.max_scope_depth`; nonnegative maximum depth, roots are depth zero |
| `SANDPOST_LOG_LEVEL` | `info` | Overrides `sandpost.log_level`; tracing filter, e.g. `info,sandpost_application=debug` |


All entry points read the optional `~/.sandpost/config.toml` (`%USERPROFILE%\.sandpost\config.toml` on Windows); `SANDPOST_CONFIG` selects another file. The discovered default file may be absent, but a file named explicitly by `SANDPOST_CONFIG` must exist. Environment variables override TOML values, which override defaults. Listener configuration uses these fields:

```toml
[sandpost]
log_level = "info"
max_scope_depth = 2

[http]
bind_address = "127.0.0.1"
port = 8025

[frontend]
# proxy_url = "http://frontend:8080"

[smtp]
bind_address = "127.0.0.1"
port = 1025
auth = "auto"
tls = "auto"
tls_mode = "starttls"
# tls_certificate = "certs/smtp-chain.pem"
# tls_private_key = "certs/smtp-key.pem"

[imap]
enabled = true
bind_address = "127.0.0.1"
port = 1143
tls = "auto"
tls_mode = "implicit"
# tls_certificate = "certs/imap-chain.pem"
# tls_private_key = "certs/imap-key.pem"
# host = "imap.example.test"
max_connections = 64

[storage]
data_directory = ""
database_path = "sandpost.sqlite3"
```

Global application settings live in `[sandpost]`. `max_scope_depth` is a nonnegative
integer: `0` allows only root scopes, and omission defaults to `2` (root at depth
zero, children at one, grandchildren at two). Set a larger value for deeper trees.
Startup validates persisted scopes against this limit, so an existing deeper tree
requires an explicit larger limit; no scopes are deleted or rewritten.
`log_level` defaults to `"info"` and accepts tracing filters, including `off`,
`error`, `warn`, `info`, `debug`, `trace`, or per-target directives such as
`"info,sandpost_application=debug"`. Each corresponding environment variable
overrides its TOML field independently. Unknown keys and invalid types fail
configuration parsing; the CLI also rejects malformed tracing filters at startup.
`[frontend].proxy_url` is optional and accepts an HTTP or HTTPS origin with a DNS
name or IP address, using port 80 or 443 by default, with root path only and no
credentials, query, or fragment. HTTPS validates the certificate and hostname
against the system trust store. The CLI
`--frontend-proxy-url` option overrides this TOML value. No environment variable
configures the frontend proxy.

HTTP, SMTP, and IMAP resolve each bind address and port independently from environment variables, TOML, then defaults. SMTP listener settings are never stored in the database or changed through management APIs. The SMTP API reports `configuration` as its listener source, the configured address, and the actual bound address. Without an attached runtime, its address and listener status are null. Port `0` is supported for all listeners and reports the allocated port.

SMTP security settings can be configured in TOML or independently overridden by their environment variable:

| Environment variable | TOML field | Allowed values | Meaning and default |
| --- | --- | --- | --- |
| `SANDPOST_SMTP_AUTH` | `smtp.auth` | `auto`, `required`, `optional` | `auto` makes AUTH optional on loopback and required elsewhere; `required` always requires AUTH; `optional` permits unauthenticated mail on any address. Default: `auto`. |
| `SANDPOST_SMTP_TLS` | `smtp.tls` | `auto`, `required`, `off` | `auto` uses no TLS without a certificate, otherwise STARTTLS is required on non-loopback and offered on loopback; `required` requires TLS on every address and needs a certificate; `off` disables TLS. Default: `auto`. |
| `SANDPOST_SMTP_TLS_MODE` | `smtp.tls_mode` | `starttls`, `implicit` | `starttls` upgrades after the greeting; `implicit` encrypts from the first byte. Default: `starttls`. |
| `SANDPOST_SMTP_TLS_CERTIFICATE` | `smtp.tls_certificate` | File path | PEM certificate chain; default: absent. Pair it with the resolved SMTP private key. |
| `SANDPOST_SMTP_TLS_PRIVATE_KEY` | `smtp.tls_private_key` | File path | PEM private key; default: absent. Pair it with the resolved SMTP certificate. |

TOML enum values are strict canonical lowercase strings; invalid values, unknown keys, or other invalid configuration fail startup without fallback. Environment parsing retains its existing normalization. TLS `required` or `implicit` requires a certificate and private key unless TLS is `off`; a certificate and key must be supplied together after independently resolving environment overrides over TOML. With `auto`, no certificate means plaintext; with a certificate, STARTTLS is required on non-loopback addresses and offered on loopback. Implicit mode encrypts from the first byte. SMTP listener address and security settings are resolved independently from configuration. Relative TOML certificate/key paths resolve from the configuration file's directory; relative `SANDPOST_SMTP_TLS_CERTIFICATE` and `SANDPOST_SMTP_TLS_PRIVATE_KEY` paths resolve from the working directory. Absolute paths are used unchanged.

IMAP settings can be configured in TOML and each field is independently overridden by its corresponding `SANDPOST_IMAP_*` variable:

| Environment variable | TOML field | Allowed values | Meaning and default |
| --- | --- | --- | --- |
| `SANDPOST_IMAP_TLS` | `imap.tls` | `auto`, `required`, `off` | TLS policy. `auto` uses TLS when a certificate pair exists; with `starttls`, TLS is required off-loopback, and without a pair clear text is allowed only on loopback. `required` needs a pair; `off` explicitly permits clear-text passwords on any address. Default: `auto`. |
| `SANDPOST_IMAP_TLS_MODE` | `imap.tls_mode` | `implicit`, `starttls` | `implicit` encrypts from the first byte; `starttls` upgrades after the greeting. Default: `implicit`. |
| `SANDPOST_IMAP_TLS_CERTIFICATE` | `imap.tls_certificate` | File path | PEM certificate chain. Default: the resolved SMTP certificate path when both IMAP pair fields are absent. |
| `SANDPOST_IMAP_TLS_PRIVATE_KEY` | `imap.tls_private_key` | File path | PEM private key. Default: the resolved SMTP key path when both IMAP pair fields are absent. |
| `SANDPOST_IMAP_HOST` | `imap.host` | Host string | Advertised host in connection instructions; blank after trimming uses the bound address. Default: bound address. |
| `SANDPOST_IMAP_MAX_CONNECTIONS` | `imap.max_connections` | Integer, 1–10000 | Maximum concurrent IMAP sessions. Default: `64`. |

IMAP TLS policy strings in TOML must be lowercase canonical values; invalid values, unknown keys, or invalid types fail startup without fallback. Defaults apply only to omitted fields. Environment variables override their corresponding TOML fields independently. The IMAP certificate and private-key values are independently merged from environment and TOML, then must either both be present or both absent; an incomplete dedicated pair is an error and does not borrow the missing half from SMTP. If neither IMAP value is supplied, the complete resolved SMTP pair is used. Relative IMAP certificate/key paths in TOML resolve from the configuration file's directory; relative `SANDPOST_IMAP_TLS_CERTIFICATE` and `SANDPOST_IMAP_TLS_PRIVATE_KEY` paths resolve from the working directory. Absolute paths are unchanged.

The removed `SANDPOST_HTTP_LISTEN`, `SANDPOST_SMTP_LISTEN`, and `SANDPOST_IMAP_LISTEN` variables are rejected at startup with migration errors; use the corresponding bind-address and port variables, and `SANDPOST_IMAP_ENABLED=false` to disable IMAP. Disabling IMAP does not delete or modify accounts, folders, flags, or message data. The existing non-loopback plaintext guard and TLS modes are unchanged.

Bind addresses accept IPv4 or unbracketed IPv6 literals (for example `::1`); ports
must be integers from 0 to 65535. `SANDPOST_IMAP_ENABLED` accepts exactly `true` or
`false`. Binding `0.0.0.0` exposes the listener on all IPv4 interfaces and does not
provide TLS or change authentication policy. Definite address conflicts among
HTTP, enabled IMAP, and SMTP fail validation before startup;
the same port on distinct interfaces is allowed. Cross-family wildcard overlap
depends on the operating system and is checked by binding. SMTP bind errors stop startup.

`SANDPOST_DATA_DIRECTORY` overrides that value; when neither is specified, macOS uses `~/Library/Application Support/Sandpost`, Windows uses `%APPDATA%\Sandpost` (RoamingAppData), and Linux uses `$XDG_DATA_HOME/sandpost` or `~/.local/share/sandpost`. If the platform data directory cannot be determined, the fallback is `data` relative to the working directory. An empty or whitespace-only `data_directory` means the platform default; a bare `data_directory =` is invalid TOML. A relative `storage.data_directory` resolves against the directory holding the configuration file, with no tilde or environment-variable expansion. `storage.database_path` selects the SQLite file and `SANDPOST_DATABASE_PATH` overrides it: relative database paths resolve against the effective data directory, absolute paths are used unchanged, and the default is `sandpost.sqlite3` inside that directory. The database path must be a string; invalid types fail startup. The optional config is never created automatically; malformed, unreadable, or unknown fields and invalid types fail startup. Restart is required for changes. Browser preferences remain in Electron's userData directory.

The unified default does not migrate or move existing data. Existing CLI `~/.sandpost` data, relative `data/`, and Electron `SandPost` data remain where they are. Select an existing directory with `SANDPOST_DATA_DIRECTORY` or move the complete directory yourself.

Malformed configuration, a missing file named by `SANDPOST_CONFIG`, invalid persisted topology/filter, failed migrations, removed SMTP endpoint variables, search writer-lock conflicts, HTTP/IMAP bind errors, unusable SMTP or IMAP certificates (checked before storage opens), non-loopback IMAP without TLS (unless explicitly disabled), and SMTP bind errors stop startup. SQLite is authoritative; Tantivy is recovered/rebuilt as a derived index.

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
`GET /api/v1/folders/<id>/messages`; the body is `{ name, icon? }`, while the legacy exposure
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
| `GET /api/v1/smtp/server` | Global administrators: read the configuration-owned SMTP listener address, runtime status, and AUTH/TLS state; no write operation is available |
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

Health, instance status, setup authorization, first-owner setup, and login are public; static SPA routes are public and unknown API paths stay 404. Other API routes require a session; application services enforce roles and message visibility. Browser bearer tokens remain in memory; login also sets an `HttpOnly`, `SameSite=Strict` cookie for session restoration and SSE. Login accepts optional `rememberMe` (default `false` for HTTP). Without it, the cookie lasts for the browser session and the server session expires after 24 hours or a server restart. With it, the server stores only a SHA-256 token digest and the verified credential hash in SQLite, without a time expiry. The persistent cookie has a 400-day lifetime renewed by `GET /api/v1/session`; clearing cookies or exceeding that browser lifetime requires signing in again. Logout and account changes revoke durable sessions, and password hashes are revalidated on every request. Cookie-authenticated mutations require same-origin `Origin`. Electron keeps its login token in the host.

SSE has bounded notifications and no durable replay: reload on `ready` (including reconnect) or `resync`. Placement/deletion changes send `resync` without IDs; notifications revalidate session. Message events carry public IDs and internal sequences, never bodies. Vue normalizes HTTP/Electron DTOs; native raw MIME crosses IPC as Buffer then Uint8Array.

IPC channels: `sandpost:inboxes:list/create`, `sandpost:views:list/save/delete`, `sandpost:messages:list/get/raw/search/attachment/delete/move/copy/trash/seen/flags` (list accepts `viewId`/DSL `query`), `sandpost:folders:list/create/update/delete/messages`, `sandpost:trash:messages/empty`, `sandpost:admin:scopes:*`, `sandpost:admin:users:*`, `sandpost:admin:smtp:server:get`, `sandpost:admin:smtp:accesses:list/create/regenerate/update/delete`, `sandpost:admin:query:validate`, `sandpost:imap:server`, `sandpost:imap:accounts:list/get/create/update/delete/regenerate/restore`, and `sandpost:imap:folders:create/update/delete` (caller-owned accounts). Each suffix is a separate channel validated in main (IDs, roles, addresses, usernames, query lengths). Credential inputs never carry passwords; IMAP inputs are allowlisted; preload has no general invocation. Subscriptions use `sandpost:event`, without another domain event bus.

Electron production serves Vue through `sandpost://app` with local SPA fallback; renderer operations never use runtime HTTP, which remains available to external tools. NativeRuntime applies shared environment settings and defaults desktop data to Electron's platform user-data path. Main starts it before a window, retains it when macOS windows close, and awaits shutdown before quit. `wait` propagates listener failures; `nextEvent` awaits Rust broadcast without polling, lag becomes `resync`, and cancel/shutdown resolves pending receives with null. Preload subscribes after attaching listeners, signals ready after acknowledgement, and removes listeners on unsubscribe.

The shared browser/Electron frontend uses Vue I18n in Composition API mode for English (`en`), French (`fr`), Italian (`it`), Spanish (`es`), German (`de`), and Danish (`da`). Catalogs are TypeScript default exports in `apps/frontend/src/i18n/<lang>/<domain>.ts`, with matching keys and named interpolation parameters across languages. The initial language prefers `sandpost.locale` in local storage, then the first supported browser language (including regional variants), then English. A language selector is available during login/setup and in the account menu. Switching updates the interface, document `lang`, dates, numbers, and folder sorting without reloading, and persists where storage is available. English is the translation fallback. Mail content, user-defined names, protocol identifiers, and query syntax remain unchanged; backend-provided error details and diagnostics retain their original text. Catalog parity, interpolation, message compilation, locale selection, and persistence are covered by frontend tests.

The persistent Vue shell has Mail (Inbox, Trash, address inboxes, Smart Folders, manual Folders) and admin pages; its + menu creates a Smart Folder, address inbox, or manual folder. Manual folder deletion identifies exposed accounts and confirms messages return to fallback. Messages can move to folders/Inbox/Trash; Trash offers Empty Trash. Users/Scopes/SMTP appear only to Owners/Admins; IMAP appears to all users. Footer shows event state only while connecting/offline and offers language/appearance/sign-out. Narrow screens use a drawer; Cmd/Ctrl+B toggles it on desktop.

Manual folder and Smart Folder dialogs offer a searchable Lucide icon picker with a
default/reset choice. Address inboxes have an edit action for their name and icon;
the receiving address and canonical recipient filter remain unchanged. The stored
icon appears in both navigation and the message-list heading, and follows the
definition across browser and Electron sessions. Shared View icons share the View's
administrative ownership. The nullable `icon` field is accepted on Views and Web
folder bodies; null uses the kind's default. Icon names are lowercase ASCII
kebab-case, at most 64 characters. Unknown names remain stored and render the
kind's default, allowing catalog changes without breaking older definitions.

Folder/search are encoded in `/mail?folder=<view id>&q=<search>` and preserved on `/mail/<message id>`. The dense two-line list supports All/Unread, refresh, arrows and j/k; selection supports Shift ranges, Cmd/Ctrl-click, and Cmd/Ctrl+A over loaded rows. Delete/Backspace, swipe, header, and inspector actions move mail to Trash immediately; inside Trash they delete permanently after confirmation. Bulk DELETE uses bounded concurrency; partial failures remain selected for retry. Selection clears on folder/search change. Keyset paging loads near the end with an explicit “Load older messages” control. Inspector renders isolated HTML/plain text; Headers copies all or downloads `.eml`; Attachments downloads decoded MIME parts. Dialogs are accessible; narrow layouts switch between list and inspector.

Search spans readable mail; Cmd/Ctrl+K or `/` focuses it. It accepts canonical DSL, shortcuts, or text. Shortcuts: `from:`, `to:`, `cc:`, `subject:`, `has:attachment`, `after:YYYY-MM-DD`, `before:YYYY-MM-DD`; prefix `-` negates. They become canonical predicates (`from:x` matches `from.address` or `envelope.from.address`; dates use `received_at` bounds at local midnight); remaining words form one `content contains` literal. Quoted whole input is literal. Shortcuts are search-only; saved Views/scopes use DSL. Suggestions preview parsing/matches, Tab completes, arrows move, Enter opens/highlights or shows all, Escape closes then clears. Opening a result selects its receiving-address inbox or Inbox fallback.

The shared CodeMirror 6 editor serves Scopes, Smart Folders and Independent account smart folders. It shows line numbers, active line, validation/cursor status; highlights query words; completes grammar-valid fields/operators/boolean values/and/or, auto-closes quotes and `header[""]`, and supports Tab/Enter acceptance, Escape, and hover documentation. Server validation remains authoritative; errors underline the reported position, collection `!=` warns, and a collapsible reference lists fields/operators/examples. Smart Folders are named canonical-query Views: personal by default; Owners/Admins may share. Views never grant access.

Admin pages: `/admin/scopes` shows hierarchy, inherited filters, live newest-match preview, sibling order, assignments (owners omitted; All-mail users flagged). `/admin/smtp-access` displays the configuration-owned global listener address and status/AUTH in read-only form, displays credentials, and supports copy username, rotate, enable/disable, confirmed delete. Create asks name/optional username. One-time credential dialog shows host/port/user/password with copy, warns rotation invalidates old credentials, and cannot close until “I have saved my credentials” is checked; plaintext leaves component state and is never stored in browser storage/toasts. Users can be created/edited/deleted with role and All/Scoped access; editing signs the account out.

SMTP Access also exposes “Authentication settings” for TLS/mechanism rules and warns when TLS-only access lacks a certificate.

`/imap` lists the caller's accounts with Mirror/Independent mode and supports create,
configure, password rotation, enable/disable and confirmed deletion. `/imap/<id>`
shows connection setup, non-destructive mode switching, mailbox previews and account-local
smart folder CRUD only in Independent. Mirror shares primary organization; Independent
preserves local folders, read state and deletions. Mode transitions require client
reconnection; deleting a credential permanently removes its Independent state.
One-time credentials remain ephemeral. Notices explain disabled service, clear-text
password risk and manual self-signed certificate trust. Full behavior: [imap.md](imap.md).

Default Inbox is virtual with nil UUID; address inboxes are personal Views for normalized, complete recipient addresses on any domain. The creation form accepts the full email address (for example `payments+test@example.com`), without appending a domain. Addresses are trimmed and ASCII-lowercased; supported addresses use an ASCII dot-atom local part (1–64 characters) and domain labels (1–63 letters/digits with internal hyphens), up to 254 characters overall. Single-label development domains are allowed. Existing `@sandpost.local` inboxes remain valid. Creating an address inbox neither provisions SMTP delivery nor moves/copies mail; it filters current authorized mail by `envelope.to.address` and deleting the inbox preserves its messages. Search indexes normalized facts in Tantivy as a disposable candidate index; canonical verification and current SQL revision/authorization checks prevent stale hits from exposing changed mail. Short substrings/negative queries may scan broadly and indexing is eventual. See [query-language.md](query-language.md), [scope-engine.md](scope-engine.md), and [storage.md](storage.md) for those contracts. Read status is authoritative primary server state, shared with Mirror accounts; Independent read state stays local to its account. Inbox badge counts newest-first until 1,000 unviewed or end: exact through 999, then `999+`, hidden at zero; it updates on opens, events and deletions. Appearance follows system until changed, then persists per device. Roles: Owner full authority; Admin manages users (except owners), scopes, SMTP access, shared Views and search maintenance; Member reads/deletes visible mail and keeps personal Views; Viewer reads visible mail and keeps personal Views. All mail access reads all mail; Scoped reads assigned scopes and inherited filters, with personal filters further restricting; no assignments means no mail. Owners bypass visibility restrictions. There is no outbound delivery; IMAP reads captured mail through users' accounts.

## Verify

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
cargo run --locked -p sandpost-test-runner -- smoke-test
yarn workspace @sandpost/frontend build
yarn workspace @sandpost/frontend test
yarn workspace @sandpost/vite-plugin-hot-file check
yarn workspace @sandpost/vite-plugin-hot-file test
cargo test --locked -p sandpost --bin sandpost --test frontend_proxy_cli
yarn workspace @sandpost/desktop check
yarn workspace @sandpost/desktop test
yarn workspace @sandpost/desktop native:build
node scripts/native-smoke.cjs
yarn workspace @sandpost/desktop smoke
yarn workspace @sandpost/desktop smoke --dev
cargo build --workspace --locked
cargo run --locked -p sandpost-test-runner -- smoke-test --frontend apps/frontend/dist
```

`cargo test --workspace --locked` and `yarn workspace @sandpost/frontend test` run the suites. The `sandpost-test-runner` crate provides `sandpost-test-runner smoke-test`: it starts the binary with isolated configuration, ports and temporary data, sends SMTP, checks MIME, HTTP authorization/search/raw/SSE, then uses independent Rust wire clients for IMAP login, LIST/SELECT/FETCH/SEARCH/STORE/IDLE push/MOVE and verifies restart persistence, UIDVALIDITY, UIDs and flags. It also checks startup summaries, bind/storage failures, port zero and offline search maintenance. No Python interpreter is required. The default server executable is `sandpost` beside the runner; `--binary PATH` selects another build. `--frontend apps/frontend/dist` additionally checks embedded assets, deep SPA routes and API 404 boundaries against that directory; build the frontend and then rebuild Rust before running it. The flag supplies expected bytes for verification and does not configure the server. Run from a source checkout so migration filenames can be compared with the SQLite history.

The Rust smoke runner reports numbered phases to stderr with `[RUN ]`, `[PASS]` or `[FAIL]`, per-phase duration and total elapsed time. An error or assertion failure marks the active phase failed and preserves the original diagnostic. Colors require an interactive terminal, no `NO_COLOR`, and `TERM` other than `dumb`; redirected/CI output stays plain text with one line per status. Checks still run in a separate server process with temporary configuration/data and dedicated loopback ports; cleanup kills/reaps unfinished children and removes temporary files on failure too.

The desktop workspace's `check` and `test` cover main/preload typing and IPC/security. After building the native addon, `node scripts/native-smoke.cjs` exercises real SMTP, events, reads, Buffer fidelity, HTTP parity, cancellation, shutdown and restart. Electron smoke sends real MIME and checks notifications, list/detail/raw IPC, exact bytes, Vue UI, HTML isolation and blocked HTTP API. Production uses the local app protocol; dev permits Vite/HMR but blocks `/api`. For unpacked app use `yarn workspace @sandpost/desktop smoke --packaged <absolute executable path>` (macOS executable: `SandPost.app/Contents/MacOS/SandPost`).

`yarn workspace @sandpost/desktop smoke:preview` exercises inline inspection through SMTP, preload, Vue and Chromium; add `--dev` or `--packaged <absolute executable path>`. It checks real DOM/inline CSS, blocked script/Node/preload/storage, exclusive DevTools target, one window, resize bounds, themed first frame/readiness loader, live/reopened light/dark theme, toolbar close/reopen, stable normal/malformed HTML inspection, minimum size, and teardown on Plain text/unmount. Because MIME may provide HTML for text/plain, it explicitly selects Plain text mode. It checks Elements selection, computed style, box model, minimize/restore. `SANDPOST_PREVIEW_CAPTURE=1` saves DevTools/email captures and loader frame in the system temp directory. Frontend/desktop tests also cover browser fallback, lifecycle, theme/toggle races, clipping/geometry, IPC policy/zoom, destruction and cleanup.

Manual inspection: test normal and malformed styled/table HTML; Inspect, select via native picker, inspect Styles/Computed/box model and Console/Sources/Network. Drag divider to limits; resize/zoom/minimize/restore; close/reopen; change HTML while open; switch Plain text/Headers, routes and folders; open menus/dialogs and scroll to verify clipping/restoration. Repeat with scripts, handlers, remote images, forms, popups and navigation and confirm they stay blocked. Browser preview has no Inspect.

Local migration validation covered macOS ARM64 native debug/release, Vite dev, production assets and unsigned `.app`. The CLI release workflow's Linux and macOS x64 targets remain unvalidated until its CI builds complete; Windows builds are temporarily disabled. Desktop installer images are not built by that workflow; signing and notarization are not configured.

## Architecture and operational limits

- [Architecture and component boundaries](architecture.md); [storage contract and SQLite migrations](storage.md); [query grammar and semantics](query-language.md); [scope authorization and index synchronization](scope-engine.md); [IMAP behavior and limits](imap.md); [code style](code-style.md).
- Implemented: one global authenticated SMTP catcher and mail pool; canonical users/roles/mail access; hierarchical scopes; personal/shared Views; SQLite storage contract and backend; typed DSL; durable search outbox/disposable Tantivy generations; MIME normalization; HTTP/SSE; optional SPA; shared browser/mock/Electron clients and embedded runtimes; users' IMAP accounts with IMAP4rev1 IDLE, UIDPLUS, MOVE, UNSELECT, NAMESPACE, ID, CHILDREN, LITERAL-, SASL-IR, STARTTLS/implicit TLS, virtual mailboxes and persistent UID state; manual folder exposure and per-message placement.
- Limits: search sync is eventual (10 ms poll, 50 ms quiet, max five-second wait; shutdown drains); broad negative/short substring searches may scan widely. Outbound mail, durable SSE replay, disk-backed attachments, advanced mail-management features are absent. IMAP APPEND, server Drafts/Sent, CONDSTORE/QRESYNC, LIST-EXTENDED and IMAP4rev2 are absent; IMAP SEARCH scans selected mailbox rather than Tantivy. Apple Mail/Thunderbird/Outlook lack automated GUI verification. SMTP TLS requires configured cert/key (no generation, upload or hot reload); without it SMTP AUTH is clear text. HTTP has no built-in TLS.
- SQLite uses one ordered script-name migration sequence with execution batches (no separate schema versions); startup validates the applied prefix and physical schema, then atomically applies missing scripts/history as one new batch. `sandpost migrate [up|status|rollback]` applies pending scripts, reports batches, or atomically reverses only the latest batch without first upgrading; mutating commands require the instance to be stopped. The 46-script pre-release baseline creates current tables directly, without historical ALTERs or data backfills; migration 0047 appends remembered sessions. Earlier development databases require explicit recreation. Down scripts remove data in reverted tables. After release, append future scripts with matching inverses. Validated name-only histories become batch 1; incompatible older pre-release formats remain unsupported and fail without clearing data. No active message-to-scope materialization; authorization composes mail access, assignments/inherited filters and personal filters. Mirror exposes every primary normal/smart folder; Independent initializes primary normal organization once and owns its own smart definitions. Smart results and global search include filed mail and exclude primary Trash. Reference-safe collection releases physical content only after all views release it.
- Search status/rebuild are administrator HTTP operations and offline CLI commands. HTTP rebuild allows SMTP ingestion; CLI status reads SQL without writer lock, CLI rebuild requires the installation lock. Index schema/checksum validation precedes recovery; indexing worker failure stops supervised listeners. In-flight work can finish with already-read facts; later requests re-read roles/policies. Credential-bound sessions are invalidated on password change, and administrators cannot grant authority they lack. SMTP listener settings come only from process configuration and require a restart to change; the interface exposes them read-only.
- Refresh waits for its captured committed index watermark, then rechecks visibility/session before IDs; SSE `ready` also waits. SMTP acknowledgement depends on SQL commit and direct reads remain immediately available. Shutdown releases index locks even if handles survive; closed writers reject mutations/wake waits. Temporary coordinators fence rebuild against outbox pruning and retry or fail under sustained acknowledgement churn.
- Administrators cannot edit credentials or delete users whose authority exceeds their own; this prevents privilege escalation by credential reset. Owners can manage all accounts.
