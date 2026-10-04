use sandpost_mail::parse_message;
use sha2::{Digest, Sha256};

const MULTIPART_MESSAGE: &str = "From: Alice <Alice@Example.COM>\r\nTo: Bob <Bob@Example.COM>\r\nCc: Carol@Example.COM\r\nSubject: Hello\r\nX-Tag: one\r\nX-Tag: two\r\nX-Folded: =?UTF-8?Q?caf=C3=A9?=\r\n\tcontinued\r\nMessage-ID: <id@example.com>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=boundary\r\n\r\n--boundary\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nbody\r\n--boundary\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=note.txt\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n--boundary--\r\n";

/// Verify mailbox normalization, header unfolding, body parsing, and attachment hashing.
#[test]
fn parse_normalizes_addresses_preserves_headers_and_hashes_attachments() {
    let raw_message = MULTIPART_MESSAGE.as_bytes().to_vec();
    let message = parse_message(
        Some("Sender@Example.COM"),
        &["RCPT@Example.COM".into()],
        raw_message.clone(),
        123,
    )
    .unwrap();
    assert_eq!(
        message.facts.envelope_from.unwrap().address,
        "sender@example.com"
    );
    assert_eq!(message.facts.envelope_to[0].domain, "example.com");
    assert_eq!(message.facts.from[0].address, "alice@example.com");
    assert_eq!(message.facts.to[0].address, "bob@example.com");
    assert_eq!(message.facts.carbon_copy[0].address, "carol@example.com");
    assert_eq!(message.facts.subject, "Hello");
    assert_eq!(message.facts.text, "body");
    assert_eq!(message.facts.headers["x-tag"], ["one", "two"]);
    assert_eq!(message.facts.headers["x-folded"], ["café continued"]);
    assert_eq!(message.raw_message, raw_message);
    assert_eq!(message.facts.received_at, 123);
    assert_eq!(message.facts.attachment_count, 1);
    let attachment = &message.attachments[0];
    assert_eq!(attachment.filename.as_deref(), Some("note.txt"));
    assert_eq!(attachment.content_type, "text/plain");
    assert_eq!(attachment.size, 5);
    assert_eq!(
        attachment.content_hash,
        format!("{:x}", Sha256::digest(b"hello"))
    );
}

/// Verify malformed mailboxes and messages over the byte limit are rejected.
#[test]
fn parse_rejects_invalid_mailboxes_and_oversize_messages() {
    assert!(parse_message(Some("invalid"), &[], b"Subject: x\r\n\r\nx".to_vec(), 0).is_err());
    assert!(parse_message(None, &[], vec![0; 10 * 1024 * 1024 + 1], 0).is_err());
}

/// Verify headerless input parses with an empty extracted body and header-only input stays empty.
#[test]
fn parse_accepts_missing_headers_or_body() {
    let headerless_message = parse_message(None, &[], b"body without headers".to_vec(), 0).unwrap();
    assert!(headerless_message.facts.text.is_empty());
    assert_eq!(headerless_message.raw_message, b"body without headers");

    let message = parse_message(None, &[], b"Subject: empty\r\n\r\n".to_vec(), 0).unwrap();
    assert_eq!(message.facts.subject, "empty");
    assert!(message.facts.text.is_empty());
}

/// Verify alternative MIME bodies, repeated addresses, and envelope separation.
#[test]
fn parse_extracts_alternative_bodies_and_separates_envelope_headers() {
    let raw_message = b"From: Header Sender <header@example.com>\r\nTo: First <first@example.com>, second@example.com\r\nCc: copy@example.com\r\nX-Tag: one\r\nX-Tag: two\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=alternative\r\n\r\n--alternative\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nplain body\r\n--alternative\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<strong>markup body</strong>\r\n--alternative--\r\n";
    let envelope_recipients = vec!["envelope@example.net".to_owned()];
    let message = parse_message(
        Some("envelope-sender@example.net"),
        &envelope_recipients,
        raw_message.to_vec(),
        0,
    )
    .unwrap();

    assert_eq!(
        message.facts.envelope_from.unwrap().address,
        "envelope-sender@example.net"
    );
    assert_eq!(message.facts.envelope_to[0].address, "envelope@example.net");
    assert_eq!(message.facts.from[0].address, "header@example.com");
    assert_eq!(
        message
            .facts
            .to
            .iter()
            .map(|mailbox| mailbox.address.as_str())
            .collect::<Vec<_>>(),
        ["first@example.com", "second@example.com"]
    );
    assert_eq!(message.facts.carbon_copy[0].address, "copy@example.com");
    assert_eq!(message.facts.text, "plain body");
    assert_eq!(message.facts.markup_body, "<strong>markup body</strong>");
    assert_eq!(message.facts.headers["x-tag"], ["one", "two"]);
}

/// Characterize recovery of an unterminated multipart and malformed quoted-printable data.
#[test]
fn parse_recovers_incomplete_multipart_and_malformed_encoding() {
    let incomplete_multipart = b"Content-Type: multipart/mixed; boundary=unfinished\r\n\r\n--unfinished\r\nContent-Type: text/plain\r\n\r\nrecovered body";
    let recovered_message = parse_message(None, &[], incomplete_multipart.to_vec(), 0).unwrap();
    assert!(recovered_message.facts.text.is_empty());
    assert_eq!(recovered_message.raw_message, incomplete_multipart);

    let malformed_encoding =
        b"Content-Type: text/plain\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nhello=4Z";
    let decoded_message = parse_message(None, &[], malformed_encoding.to_vec(), 0).unwrap();
    assert_eq!(decoded_message.facts.text, "hello=4Z");
}
