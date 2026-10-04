# sandpost-mail

`sandpost-mail` parses submitted MIME messages and provides a bounded local SMTP
receiver. It converts a message into the shared `sandpost_core::Message` model;
the application supplies the persistence callback.

## Message flow

The SMTP server accepts `HELO`/`EHLO`, `MAIL`, `RCPT`, `DATA`, `RSET`, `NOOP`
and `QUIT`. It validates the envelope and limits, reads CRLF-terminated DATA,
removes SMTP dot-stuffing, and passes the resulting message bytes to
`parse_message`. Envelope sender/recipients remain distinct from MIME `From`,
`To` and `Cc` headers. Mailbox addresses and domains are lowercased during
normalization.

The existing `mail-parser` library extracts decoded text and HTML bodies,
headers, message ID and attachment metadata. Parsing accepts recoverable MIME
input; success does not promise strict MIME validation. Repeated header values
are retained, header names are lowercased, and folded header lines are unfolded.
The original message bytes are retained after SMTP dot-unstuffing. Attachments
are represented by filename, content type, decoded size and SHA-256 hash; their
bytes are not saved as separate blobs.

The public parsing API can be used without starting a listener:

```rust
use sandpost_mail::parse_message;

/// Parse a submitted message without starting the SMTP service.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let message = parse_message(
        Some("sender@example.com"),
        &["recipient@example.com".to_owned()],
        b"From: sender@example.com\r\nTo: recipient@example.com\r\nSubject: Hello\r\n\r\nLocal test mail.\r\n".to_vec(),
        1,
    )?;
    assert_eq!(message.facts.subject, "Hello");
    Ok(())
}
```

## SMTP service contract

`serve` accepts a Tokio `TcpListener`, a handler, and a shutdown future. The
handler receives each parsed message and must finish persistence before
returning `Ok(())`; only then does SMTP return `250 message accepted`. A handler
error or timeout returns a temporary SMTP failure. The handler is awaited with
a 60-second timeout. Timing out or aborting a session cannot roll back side
effects the handler has already made in an external system.

This is a local development catcher. It has no TLS, SMTP AUTH, or forwarding.
ESMTP parameters are rejected. The current transport bounds are:

| Bound | Limit |
| --- | ---: |
| Concurrent connections | 32 |
| Accepted recipients per transaction | 100 |
| Protocol line, including CRLF | 1,000 bytes |
| Message DATA after dot-unstuffing | 10 MiB |
| SMTP read/write and handler timeout | 60 seconds |
| Graceful session drain at shutdown | 5 seconds |

An overlong or malformed command line ends that session. An oversized DATA
message receives `552` and closes the session. Excess connections are closed
immediately. Shutdown stops accepting connections, drains active sessions for
up to five seconds, then aborts remaining sessions.

## Modules

- `lib.rs` exposes `parse_message`, `serve` and `MailError`.
- `message_parsing.rs` uses `mail-parser` to extract message facts, normalize
  mailboxes and unfold header values.
- `server.rs` accepts connections, enforces the connection bound and drains
  sessions during shutdown.
- `session.rs` handles SMTP commands, transaction state and the persistence
  callback.
- `protocol_stream.rs` reads bounded CRLF lines and DATA, removes dot-stuffing
  and writes responses.
- `limits.rs` defines shared size, count and timeout bounds.

## Verification

Internal tests in `src/tests/` exercise protocol helpers and transaction state.
Integration tests in `tests/` import only the public crate interface. This uses
the same directory convention as `sandpost-query`.

From the workspace root, run all crate tests with
`cargo test -p sandpost-mail --locked`. Session protocol tests live in
`src/tests/session.rs`; line framing, DATA boundaries and paused-clock timeout
tests live in `src/tests/protocol_stream.rs`. `tests/message_parsing.rs` covers public message parsing,
normalization, raw-byte preservation and attachment metadata. `tests/server.rs`
checks the connection ceiling and shutdown drain using real local sockets.
The README's Rust example is also compiled as a documentation test. Workspace checks
are listed in the root [README](../../README.md).
