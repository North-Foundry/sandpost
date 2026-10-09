# sandpost-smtp

`sandpost-smtp` is Sandpost's bounded SMTP receiver with SMTP AUTH. It frames
the protocol and leaves every decision about credentials and storage to the
application, which implements `SessionHandler`. Received messages are parsed
with `sandpost-mime` into the shared `sandpost_core::Message` model.

```rust,no_run
use sandpost_core::Message;
use sandpost_smtp::{
    AuthenticationError, AuthenticationOutcome, AuthenticationPolicy, Credentials, DeliveryError,
    Limits, SessionHandler, SmtpServer,
};

/// Accept one fixed credential and print every delivered subject.
struct Capture;

impl SessionHandler for Capture {
    type Principal = String;

    /// Identify a session by its username when the password matches.
    async fn authenticate(
        &self,
        credentials: Credentials,
    ) -> Result<AuthenticationOutcome<String>, AuthenticationError> {
        Ok(if credentials.password == "secret" {
            AuthenticationOutcome::Authenticated(credentials.username)
        } else {
            AuthenticationOutcome::InvalidCredentials
        })
    }

    /// Persist the message before the server acknowledges it.
    async fn deliver(
        &self,
        message: Message,
        principal: Option<String>,
    ) -> Result<(), DeliveryError> {
        println!("{principal:?} sent {:?}", message.facts.subject);
        Ok(())
    }
}

/// Serve SMTP on loopback until Ctrl+C.
#[tokio::main]
async fn main() -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:1025").await?;
    SmtpServer::new(Capture)
        .authentication(AuthenticationPolicy::Optional)
        .limits(Limits {
            maximum_connection_count: 8,
            ..Limits::default()
        })
        .serve(listener, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    Ok(())
}
```

## API

- `SmtpServer::new(handler)` starts from safe defaults (authentication
  `Required`, `TransportSecurity::Plaintext`, `Limits::default()`);
  `authentication`, `transport_security`, `server_name` and `limits` adjust them, and
  `serve(listener, shutdown)` runs until the shutdown future completes.
  `server_name` validates and lowercases the domain or address literal used in
  greetings and trace fields, and as the domain for an unqualified Postmaster
  recipient; it defaults to `sandpost.localhost`.
- `TransportSecurity` is `Plaintext`, `StartTls { configuration, required }`
  (RFC 3207) or `Implicit { configuration }` (RFC 8314, TLS from the first
  byte). `TlsConfiguration::from_pem_files(certificate_chain, private_key)` (or
  `from_pem`) loads a PEM certificate chain and key with rustls; failures are a
  `TlsConfigurationError`.
- `SessionHandler` has two operations, each bounded by `Limits::handler_timeout`:
  - `authenticate` receives the `Credentials` presented through AUTH, including
    the `mechanism` used and whether the session is `encrypted`, and returns an
    `AuthenticationOutcome`: `Authenticated(principal)` (`235`),
    `InvalidCredentials` (`535`, counted toward the failure limit),
    `EncryptionRequired` (`538 5.7.11`) or `MechanismNotAllowed` (`534 5.7.9`)
    for per-credential rules, which do not count as failures. A temporary
    verification failure is `AuthenticationError::Unavailable` (`454`, not
    counted either).
  - `deliver` receives each parsed message with the session's principal (or none)
    and must finish persistence before returning `Ok(())`; only then does SMTP
    return `250 message accepted`. `DeliveryError::Rejected` is reported as a
    permanent `554` (for example, credentials revoked after authentication) and
    `DeliveryError::Temporary` or a timeout as a temporary `451`.
- `Limits` is a plain struct with public fields; override what you need with
  struct update syntax.

Timing out or aborting a session cannot roll back side effects the handler has
already made in an external system. `Credentials` implements neither `Debug` nor
`Display`, so passwords cannot be formatted into logs.

## TLS

With `StartTls`, EHLO lists `STARTTLS` on plaintext connections. The command
answers `220 2.0.0` and performs the handshake; afterwards the client must greet
again, and the plaintext greeting, principal, failure count and transaction are
forgotten. If the client sent anything after `STARTTLS` before the handshake,
those bytes are never executed: the server answers `421` and closes (the
STARTTLS command-injection defence). `STARTTLS` without TLS configured receives
`502`, on an encrypted connection or during a transaction `503`. With
`required: true`, EHLO does not list AUTH before the upgrade, `AUTH` receives
`538 5.7.11` and `MAIL FROM` receives `530 5.7.0 must issue a STARTTLS command
first`. With `Implicit`, the handshake completes before the `220` greeting and
STARTTLS is never offered. A handshake that does not finish within
`Limits::tls_handshake_timeout` disconnects the client. Client certificates are
not requested.

## Protocol behavior

This is a bounded SMTP capture server implementing selected SMTP extensions;
RFC references below describe the implemented behavior, not universal
compliance with those specifications. It accepts `HELO`/`EHLO`, `AUTH`, `STARTTLS`, `MAIL`, `RCPT`, `DATA`,
`RSET`, `NOOP`, `QUIT`, `VRFY` (answered `252` without confirming mailboxes) and
`HELP` (`214`). EHLO advertises `PIPELINING`, `SIZE <limit>`, `8BITMIME`,
`ENHANCEDSTATUSCODES`, `STARTTLS` when available and `AUTH PLAIN LOGIN` when
allowed. It implements bounded capture behavior from RFC 5321, `SIZE` (RFC 1870),
`8BITMIME` (RFC 6152), AUTH framing (RFC 4954) with `PLAIN` (RFC 4616), the
de-facto `LOGIN` mechanism, STARTTLS (RFC 3207), implicit TLS (RFC 8314),
enhanced status codes (RFC 2034), and generated trace fields (RFC 3848). Replies
carry enhanced status codes. Unknown commands receive
`500 5.5.2`, standard ones that are deliberately absent (`EXPN`, `TURN`, `ETRN`,
`BDAT`…) `502 5.5.1`, and malformed ones `501 5.5.4` (for example `EHLO` without
a domain, or `DATA`, `RSET` and `QUIT` with an argument). `MAIL FROM` accepts
`SIZE=` (RFC 1870; a declared size over the limit receives `552 5.3.4` before
any data), `BODY=7BIT|8BITMIME` (RFC 6152) and `AUTH=` (RFC 4954, validated but
not used for authorization); duplicate parameters and malformed values are
rejected. Other parameters on `MAIL FROM` or `RCPT TO` receive `555 5.5.4`. SMTP
envelope paths are strict ASCII RFC 5321 mailbox syntax, including quoted local
parts and address literals; obsolete source routes are validated and discarded.
Local-part spelling and case are preserved and only the domain is lowercased.
The special bare `RCPT TO:<Postmaster>` recipient resolves to
`Postmaster@<server_name>`, preserving the supplied local-part spelling. An invalid sender receives `501 5.1.7`, an invalid
recipient `501 5.1.3`. Command names are case-insensitive and may be preceded by
ASCII whitespace. A valid `HELO` or `EHLO` is required before `MAIL`; AUTH and
ESMTP parameters require EHLO. `MAIL FROM` while a transaction is open receives
`503 5.5.1` (send `RSET` first). SMTP DATA is CRLF-terminated and dot-unstuffed
before parsing. The accepted client message is size-checked before the server
prepends its `Return-Path` and `Received` trace headers; prior Return-Path fields
are removed, while existing Received fields remain. The captured raw bytes
include these trace headers, and the submitted body and attachment bytes remain
unchanged. A message that fails MIME parsing is refused with `550`.

EHLO advertises `AUTH PLAIN LOGIN` (RFC 4954, RFC 4616 for PLAIN). Only mechanisms that
transmit the password are offered, because applications store one-way password
hashes; challenge-response mechanisms such as CRAM-MD5 receive `504`. AUTH
requires EHLO, is refused during a mail transaction or after a successful AUTH
(`503`), accepts `*` to cancel, and rejects undecodable responses (`501`). A
PLAIN authorization identity other than the username is never permitted.
Base64 responses use strict standard padded syntax and AUTH continuation lines
have their own bound. MAIL's AUTH parameter uses strict xtext validation.
PLAIN supports 255 bytes per UTF-8 field; username and password must be nonempty
and NUL-free. Unicode preparation is left to the application's authentication
database rather than imposed by the protocol layer.
Invalid credentials receive `535 5.7.8`; reaching the failure limit receives
`421` and closes the session. With `AuthenticationPolicy::Required`, `MAIL FROM`
before a successful AUTH receives `530 5.7.0`; with `Optional`, unauthenticated
sessions may still submit mail and are delivered without a principal. The
authenticated principal survives `RSET` and a repeated `EHLO`.

Accept errors never stop the server: a failure of the single connection being
accepted (refused, aborted, reset, interrupted) is skipped, and any other error,
such as running out of file descriptors, is logged and retried after a
one-second pause that shutdown interrupts. Shutdown stops accepting and releases
the port at once, then drains sessions for a bounded period before aborting
remaining async sessions. A MIME parse already running on Tokio's blocking pool
cannot be cancelled; it retains its concurrency lease and can finish after
`serve` returns. Cancellation or timeout of an application handler also cannot
undo external side effects already performed and does not guarantee exactly-once
delivery.

This is a development catcher: it never forwards mail. Without TLS, AUTH
credentials travel in clear text, so configure `StartTls { required: true }` or
`Implicit` for listeners outside loopback. `SMTPUTF8` (non-ASCII addresses),
`CHUNKING`/`BDAT`, DSN, `EXPN`, `TURN`, `ETRN`, `ATRN`, `BURL`, outbound delivery,
and client-certificate authentication are not implemented. Support is limited
to the commands, extensions, reply behavior, and bounds described here; it is
not a claim of complete compliance with RFC 5321 or any other referenced RFC.

| Bound (`Limits` field) | Default | When exceeded |
| --- | ---: | --- |
| Message size after dot-unstuffing (`maximum_message_size`) | 10 MiB | `552 5.3.4`, session closed |
| Recipients per transaction (`maximum_recipient_count`) | 100 | `452` |
| Concurrent connections (`maximum_connection_count`) | 32 | `421 4.3.2`, connection closed |
| Failed AUTH attempts (`maximum_authentication_failures`) | 3 | `421`, session closed |
| Network read/write (`input_output_timeout`) | 300 seconds | `421`, session closed |
| Handler call (`handler_timeout`) | 60 seconds | `454` or `451` |
| TLS handshake (`tls_handshake_timeout`) | 30 seconds | connection closed |
| Session lifetime (`maximum_session_duration`) | 15 minutes | `421 4.4.2`, session closed |
| Drain at shutdown (`shutdown_drain_timeout`) | 5 seconds | sessions aborted |

Ordinary commands are limited to 512 bytes including CRLF; an extended `MAIL`
line may use 1,038 bytes to accommodate the RFC 1870 SIZE and RFC 4954 AUTH
allowances. AUTH continuations are limited to 12,288 bytes. Unstuffed DATA lines
are limited to 1,000 bytes including CRLF, and reply lines to 512 bytes including
CRLF. A complete ordinary command over 512 bytes receives `500` and the session
can continue. Input exceeding the physical command buffer of 1,038 bytes or
the AUTH continuation buffer receives `500` and closes; overlong DATA or
unterminated lines also close. With a zero-byte message allowance, EHLO lists
bare `SIZE` instead of `SIZE 0`, which would mean no fixed upper bound in RFC 1870.

## Modules

- `lib.rs` exposes `SmtpServer`, `SessionHandler`, `Credentials`,
  `AuthenticationPolicy`, `AuthenticationError`, `DeliveryError`, `Limits`,
  `TransportSecurity`, `TlsConfiguration` and `TlsConfigurationError`.
- `tls.rs`: certificate loading, `TransportSecurity`, and the session stream that
  STARTTLS upgrades in place.
- `server.rs`: the `SmtpServer` builder and accept loop over a `ConnectionSource`
  (a TCP listener, or a scripted source in tests), the connection bound, accept
  error recovery and shutdown draining.
- `handler.rs`: the application contract and its error types.
- `limits.rs`: `Limits` and the fixed protocol bounds.
- `framing.rs`: bounded lines, DATA with dot-unstuffing, and bounded reply writes.
- `session/mod.rs`: one conversation as a `Session` holding the open transaction
  as a `sandpost_mime::Envelope`, one method per command (including STARTTLS),
  the EHLO extension list, and the lifetime bound.
- `session/command.rs`: pure parsing of a command line into a typed `Command`,
  including `MAIL FROM` ESMTP parameters.
- `session/mailbox.rs`: SMTP mailbox, path and domain grammar.
- `session/trace.rs`: final Return-Path and Received fields and their searchable facts.
- `session/authentication.rs`: the PLAIN and LOGIN exchanges.
- `session/delivery.rs`: parsing the received message and choosing the reply from
  the handler's outcome.

## Verification

Internal tests in `src/tests/` cover command syntax (`command.rs`), framing
(`framing.rs`), transactions (`session_transaction.rs`), authentication
(`session_authentication.rs`), RFC 5321 reply classes and `SIZE`
(`session_compliance.rs`), STARTTLS, implicit TLS and the TLS requirement
(`session_tls.rs`), certificate loading (`tls.rs`), limits and timeouts with
small `Limits` (`session_limits.rs`), handler errors (`handler.rs`) and the
accept loop over a scripted connection source (`server.rs`); `support.rs` holds
the shared test handlers, in-memory client and `rcgen` self-signed certificates.
`tests/server.rs` checks the connection limit and shutdown drain, and
`tests/tls.rs` a full STARTTLS session, through the public API with real sockets.
`tests/rfc_conformance.rs` independently checks pipelined transactions, 8BITMIME
octets, SIZE accounting, enhanced replies, reset behavior and unsupported
extensions over TCP. [RFC-COVERAGE.md](RFC-COVERAGE.md) maps the 15 RFCs in the
receiver profile to enabled tests; `tests/rfc_coverage.rs` guards those links and
the registered enhanced-status assignments. The examples are
compiled as documentation tests. From the workspace root run
`cargo test -p sandpost-smtp --locked`.
