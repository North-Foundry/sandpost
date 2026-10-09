use sandpost_mime::{DEFAULT_SIZE_LIMIT, Envelope, MimeError, RawMessage, normalize_mailbox};

const MULTIPART_MESSAGE: &str = "From: Alice <Alice@Example.COM>\r\nTo: Bob <Bob@Example.COM>\r\nCc: Carol@Example.COM\r\nSubject: Hello\r\nX-Tag: one\r\nX-Tag: two\r\nX-Folded: =?UTF-8?Q?caf=C3=A9?=\r\n\tcontinued\r\nMessage-ID: <id@example.com>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=boundary\r\n\r\n--boundary\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nbody\r\n--boundary\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=note.txt\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n--boundary--\r\n";

/// Verify mailbox normalization, header unfolding, body parsing, and attachment hashing.
#[test]
fn parse_normalizes_addresses_preserves_headers_and_hashes_attachments() {
    let raw_message = MULTIPART_MESSAGE.as_bytes().to_vec();
    let message = RawMessage::new(raw_message.clone())
        .envelope(
            Envelope::new()
                .with_sender(normalize_mailbox("Sender@Example.COM").unwrap())
                .with_recipient(normalize_mailbox("RCPT@Example.COM").unwrap()),
        )
        .received_at(123)
        .parse()
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
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
}

/// Verify malformed mailboxes, oversize messages, and empty input fail with specific errors.
#[test]
fn parse_rejects_invalid_mailboxes_and_oversize_messages() {
    assert_eq!(
        normalize_mailbox("invalid"),
        Err(MimeError::InvalidMailbox("invalid".into()))
    );
    assert_eq!(
        MimeError::InvalidMailbox("invalid".into()).to_string(),
        "invalid mailbox \"invalid\""
    );
    assert_eq!(
        RawMessage::new("From: not-an-address@\r\n\r\nx")
            .parse()
            .unwrap_err(),
        MimeError::InvalidMailbox("not-an-address@".into())
    );
    let oversize = RawMessage::new(vec![0; DEFAULT_SIZE_LIMIT + 1])
        .parse()
        .unwrap_err();
    assert_eq!(
        oversize,
        MimeError::MessageTooLarge {
            size: DEFAULT_SIZE_LIMIT + 1,
            limit: DEFAULT_SIZE_LIMIT
        }
    );
    assert_eq!(
        RawMessage::new("Subject: five\r\n\r\n")
            .size_limit(4)
            .parse()
            .unwrap_err()
            .to_string(),
        "message is 17 bytes, over the 4-byte limit"
    );
    assert_eq!(
        RawMessage::new(Vec::new()).parse().unwrap_err(),
        MimeError::MissingHeaders
    );
}

/// Without an explicit arrival time the parse time is recorded, with an empty envelope.
#[test]
fn defaults_record_the_parse_time_and_an_empty_envelope() {
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let message = RawMessage::new("Subject: now\r\n\r\n").parse().unwrap();
    assert!(message.facts.received_at >= before);
    assert!(message.facts.received_at <= before + 5);
    assert!(message.facts.envelope_from.is_none());
    assert!(message.facts.envelope_to.is_empty());
}

/// The envelope builder keeps recipients in order and a null sender by default.
#[test]
fn envelope_builder_keeps_recipient_order() {
    let mailboxes = ["one@example.test", "two@example.test", "three@example.test"]
        .map(|address| normalize_mailbox(address).unwrap());
    let envelope = Envelope::new()
        .with_recipient(mailboxes[0].clone())
        .with_recipients(mailboxes[1..].iter().cloned());
    assert!(envelope.sender.is_none());
    assert_eq!(envelope.recipients, mailboxes);
}

/// A header whose raw bytes are not UTF-8 is kept and does not shift later header values.
#[test]
fn non_utf8_header_values_do_not_shift_later_headers() {
    let raw_message =
        b"X-First: one\r\nX-Binary: caf\xe9\r\nX-Last: last\r\nSubject: kept\r\n\r\nbody".to_vec();
    let message = RawMessage::new(raw_message).parse().unwrap();
    assert_eq!(message.facts.headers["x-first"], ["one"]);
    assert_eq!(message.facts.headers["x-binary"].len(), 1);
    assert_eq!(message.facts.headers["x-last"], ["last"]);
    assert_eq!(message.facts.headers["subject"], ["kept"]);
    assert_eq!(message.facts.subject, "kept");
}

/// Verify headerless input parses with an empty extracted body and header-only input stays empty.
#[test]
fn parse_accepts_missing_headers_or_body() {
    let headerless_message = RawMessage::new("body without headers").parse().unwrap();
    assert!(headerless_message.facts.text.is_empty());
    assert_eq!(headerless_message.raw_message, b"body without headers");

    let message = RawMessage::new("Subject: empty\r\n\r\n").parse().unwrap();
    assert_eq!(message.facts.subject, "empty");
    assert!(message.facts.text.is_empty());
}

/// Verify alternative MIME bodies, repeated addresses, and envelope separation.
#[test]
fn parse_extracts_alternative_bodies_and_separates_envelope_headers() {
    let raw_message = b"From: Header Sender <header@example.com>\r\nTo: First <first@example.com>, second@example.com\r\nCc: copy@example.com\r\nX-Tag: one\r\nX-Tag: two\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=alternative\r\n\r\n--alternative\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nplain body\r\n--alternative\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<strong>markup body</strong>\r\n--alternative--\r\n";
    let message = RawMessage::new(raw_message.as_slice())
        .envelope(
            Envelope::new()
                .with_sender(normalize_mailbox("envelope-sender@example.net").unwrap())
                .with_recipient(normalize_mailbox("envelope@example.net").unwrap()),
        )
        .parse()
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
    let recovered_message = RawMessage::new(incomplete_multipart.as_slice())
        .parse()
        .unwrap();
    assert!(recovered_message.facts.text.is_empty());
    assert_eq!(recovered_message.raw_message, incomplete_multipart);

    let malformed_encoding =
        b"Content-Type: text/plain\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nhello=4Z";
    let decoded_message = RawMessage::new(malformed_encoding.as_slice())
        .parse()
        .unwrap();
    assert_eq!(decoded_message.facts.text, "hello=4Z");
}
