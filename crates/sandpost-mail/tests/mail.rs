use sandpost_mail::parse_message;
use sha2::{Digest, Sha256};

const MIME: &str = "From: Alice <Alice@Example.COM>\r\nTo: Bob <Bob@Example.COM>\r\nCc: Carol@Example.COM\r\nSubject: Hello\r\nX-Tag: one\r\nX-Tag: two\r\nX-Folded: =?UTF-8?Q?caf=C3=A9?=\r\n\tcontinued\r\nMessage-ID: <id@example.com>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=boundary\r\n\r\n--boundary\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nbody\r\n--boundary\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=note.txt\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n--boundary--\r\n";

#[test]
fn parse_normalizes_addresses_preserves_headers_and_hashes_attachments() {
    let raw = MIME.as_bytes().to_vec();
    let message = parse_message(
        Some("Sender@Example.COM"),
        &["RCPT@Example.COM".into()],
        raw.clone(),
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
    assert_eq!(message.facts.cc[0].address, "carol@example.com");
    assert_eq!(message.facts.subject, "Hello");
    assert_eq!(message.facts.text, "body");
    assert_eq!(message.facts.headers["x-tag"], ["one", "two"]);
    assert_eq!(message.facts.headers["x-folded"], ["café continued"]);
    assert_eq!(message.raw_mime, raw);
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

#[test]
fn parse_rejects_invalid_mailboxes_and_oversize_messages() {
    assert!(parse_message(Some("invalid"), &[], b"Subject: x\r\n\r\nx".to_vec(), 0).is_err());
    assert!(parse_message(None, &[], vec![0; 10 * 1024 * 1024 + 1], 0).is_err());
}
