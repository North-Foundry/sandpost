# sandpost-mime

`sandpost-mime` turns captured message bytes into the shared
`sandpost_core::Message` model. It is protocol-neutral: SMTP ingestion uses it
today, and an IMAP `APPEND` would use it the same way. It does no I/O.

```rust
use sandpost_mime::{Envelope, RawMessage, normalize_mailbox};

/// Parse a submitted message together with its SMTP envelope.
fn main() -> Result<(), sandpost_mime::MimeError> {
    let message = RawMessage::new("From: App <App@Example.test>\r\nSubject: Hello\r\n\r\nHi.\r\n")
        .envelope(
            Envelope::new()
                .with_sender(normalize_mailbox("bounce@example.test")?)
                .with_recipient(normalize_mailbox("QA@Example.test")?),
        )
        .received_at(1_700_000_000)
        .parse()?;
    assert_eq!(message.facts.from[0].address, "app@example.test");
    assert_eq!(message.facts.envelope_to[0].domain, "example.test");
    Ok(())
}
```

## API

- `RawMessage` wraps the bytes and is configured fluently: `envelope`,
  `received_at` (Unix seconds; defaults to the time of parsing) and `size_limit`
  (defaults to `DEFAULT_SIZE_LIMIT`, 10 MiB). `parse` consumes it and returns a
  message with a new identifier.
- `Envelope` holds the SMTP reverse path (`None` is the null sender `<>`) and the
  forward paths as already normalized mailboxes. SMTP envelope paths are parsed
  by `sandpost-smtp` under its strict ASCII SMTP syntax: local-part spelling and
  case are preserved while the domain is lowercased. The MIME
  `normalize_mailbox` helper is not the SMTP envelope parser. The envelope is
  separate from the `From`,
  `To` and `Cc` headers, which are parsed from the message itself.
- `normalize_mailbox` trims and lowercases an address and derives its domain; it
  supports quoted local parts, quoted pairs, obsolete mixed local-part words,
  domain literals and UTF-8 address headers. ASCII case folding is a search
  normalization policy, not a statement that mailbox local parts are case insensitive.
- `attachment_bytes(raw_message, index)` returns transfer-decoded attachment
  octets in metadata order. It does not apply text charset conversion and parses
  the supplied message for each call.
- `content_reference_bytes(raw_message, reference)` resolves `cid:` and `mid:`
  URLs inside the supplied message, including percent-encoded identifiers.
  It never retrieves external resources.
- `ParsedMessage::parse(&raw_message)` keeps a reusable MIME view for reading
  several attachments or references. `attachment_count`, `attachment_bytes`
  and `content_reference_bytes` reuse the same parsed structure; `raw_message`
  returns the unchanged borrowed source. The third-party parser types stay internal.
  This view has no envelope, receipt time or ingest size limit; use `RawMessage`
  when producing a normalized message for storage.
- `MimeError` describes size, parsing, address, attachment-index and content-reference failures.

For several reads, retain one parsed view:

```rust
use sandpost_mime::ParsedMessage;

/// Read several parts without repeating the MIME parse.
fn main() -> Result<(), sandpost_mime::MimeError> {
    let raw_message = b"Message-ID: <message@example.test>\r\n\
                        Content-Type: multipart/mixed; boundary=parts\r\n\r\n\
                        --parts\r\nContent-Disposition: attachment\r\n\
                        Content-ID: <first@example.test>\r\n\r\none\r\n\
                        --parts\r\nContent-Disposition: attachment\r\n\r\ntwo\r\n\
                        --parts--\r\n";
    let parsed = ParsedMessage::parse(raw_message)?;
    let attachments = (0..parsed.attachment_count())
        .map(|index| parsed.attachment_bytes(index))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(attachments, [b"one".to_vec(), b"two".to_vec()]);
    assert_eq!(parsed.content_reference_bytes("cid:first@example.test")?, b"one");
    assert_eq!(parsed.content_reference_bytes("mid:message@example.test")?, raw_message);
    Ok(())
}
```

## Normalization

The `mail-parser` library supplies MIME decoding. The crate applies receiver
rules for unknown transfer encodings and charsets (opaque `application/octet-stream`),
the last supported plain/HTML alternative and the `multipart/related` root.
Several selected bodies are joined with newlines; when only one representation
exists, the dependency provides a plain-text or HTML conversion of that body.
Parsing accepts recoverable MIME input; success does not promise strict MIME
validation. Header names are lowercased, repeated values are retained in order,
folded lines are unfolded, and every header line is kept: when a value cannot be
decoded to text its raw bytes are used with invalid UTF-8 replaced. Mailboxes in
address headers are trimmed and lowercased independently of SMTP envelope spelling, and an address without
`local@domain` form fails the parse. The original bytes become the message's raw
form unchanged. Attachments are described by filename, content type, decoded
size and SHA-256 hash, calculated from exactly the octets returned by
`attachment_bytes`. Original attachment bytes remain in the captured message.
Missing inner multipart closing delimiters can be recovered at enclosing
boundaries in a temporary parser view. Synthetic recovery bytes never enter
stored messages, downloads or content hashes. Malformed transfer encodings
retain the dependency's recovery policy of exposing their raw body.

Parsing accepts incomplete messages and headerless input when the dependency
can recover it. It does not enforce required RFC 5322 fields or sender wire
limits. The returned HTML is extracted content; sanitizing and displaying it
belongs to the application.

## Modules

- `raw_message.rs`: the `RawMessage` builder, parsing, address and body extraction.
- `parsed_message.rs`: reusable public view for repeated attachment and reference reads.
- `envelope.rs`: `Envelope`.
- `mailbox.rs`: `normalize_mailbox`.
- `headers.rs`: header collection and unfolding.
- `parsing.rs`: shared receiver rules, body selection and original-byte offset mapping.
- `boundaries.rs`: enclosing multipart boundary recovery.
- `attachments.rs`: transfer-decoded downloads, attachment metadata and hashing.
- `content_reference.rs`: local Content-ID and Message-ID URL resolution.
- `error.rs`: `MimeError`.

## Verification

The [RFC receiver profile](RFC-COVERAGE.md) maps each listed standard to enabled
regression tests and records requirements outside the crate's role. The suite
also checks quoted and internationalized addresses, legacy charset attachment
fidelity, malformed boundary recovery, related roots, content references,
deep nesting and size limits. The application tests verify that actual downloads
use the same decoder and preserve authorization. Documentation examples run as
tests. From the workspace root run
`cargo test -p sandpost-mime --locked`.

Passing these tests verifies the documented cases; it does not certify every
requirement of every MIME-related RFC.
