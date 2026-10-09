use sandpost_mime::{MimeError, RawMessage, attachment_bytes};
use sha2::{Digest, Sha256};

/// Build a message with the supplied MIME part headers and wire bodies.
fn multipart_message(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut message = b"From: sender@example.test\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=parts\r\n\r\n".to_vec();
    for (headers, body) in parts {
        message.extend_from_slice(b"--parts\r\n");
        message.extend_from_slice(headers.as_bytes());
        message.extend_from_slice(b"\r\n\r\n");
        message.extend_from_slice(body);
        message.extend_from_slice(b"\r\n");
    }
    message.extend_from_slice(b"--parts--\r\n");
    message
}

/// Parse a message and return its attachment metadata for byte-fidelity assertions.
fn parse_message(raw_message: &[u8]) -> sandpost_core::Message {
    RawMessage::new(raw_message).parse().unwrap()
}

/// Return the lowercase SHA-256 digest for expected attachment octets.
fn expected_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Ensure text charset decoding never changes returned attachment bytes or metadata hashes.
#[test]
fn text_charset_is_not_applied_to_base64_or_quoted_printable_attachments() {
    let raw_message = multipart_message(&[
        (
            "Content-Type: text/plain; charset=iso-8859-1\r\nContent-Disposition: attachment; filename=base64.txt\r\nContent-Transfer-Encoding: base64",
            b"Y2Fm6Q==",
        ),
        (
            "Content-Type: text/plain; charset=iso-8859-1\r\nContent-Disposition: attachment; filename=quoted.txt\r\nContent-Transfer-Encoding: quoted-printable",
            b"caf=E9",
        ),
    ]);
    let expected_bytes: [&[u8]; 2] = [b"caf\xe9", b"caf\xe9"];
    let message = parse_message(&raw_message);

    assert_eq!(message.attachments.len(), expected_bytes.len());
    for (index, expected) in expected_bytes.iter().enumerate() {
        assert_eq!(attachment_bytes(&raw_message, index).unwrap(), *expected);
        assert_eq!(message.attachments[index].size, expected.len() as u64);
        assert_eq!(
            message.attachments[index].content_hash,
            expected_hash(expected)
        );
    }
}

/// Preserve arbitrary 7bit, 8bit, and binary attachment octets in part order.
#[test]
fn preserves_unencoded_octets_and_multiple_attachment_order() {
    let raw_message = multipart_message(&[
        (
            "Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=seven.bin\r\nContent-Transfer-Encoding: 7bit",
            b"ascii",
        ),
        (
            "Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=eight.bin\r\nContent-Transfer-Encoding: 8bit",
            b"caf\xe9",
        ),
        (
            "Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=binary.bin\r\nContent-Transfer-Encoding: binary",
            b"\x00\xff\x80\rX",
        ),
    ]);
    let expected_bytes: [&[u8]; 3] = [b"ascii", b"caf\xe9", b"\x00\xff\x80\rX"];
    let message = parse_message(&raw_message);

    assert_eq!(message.attachments.len(), expected_bytes.len());
    for (index, expected) in expected_bytes.iter().enumerate() {
        assert_eq!(attachment_bytes(&raw_message, index).unwrap(), *expected);
        assert_eq!(message.attachments[index].size, expected.len() as u64);
        assert_eq!(
            message.attachments[index].content_hash,
            expected_hash(expected)
        );
    }
}

/// Preserve missing metadata defaults and decode RFC 2183/2231 attachment names.
#[test]
fn handles_missing_metadata_disposition_and_extended_filename_parameters() {
    let raw_message = multipart_message(&[
        ("Content-Disposition: attachment", b"bare"),
        (
            "Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename*=UTF-8''caf%C3%A9.txt",
            b"single",
        ),
        (
            "Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename*0*=UTF-8''long%20; filename*1*=name.txt",
            b"continued",
        ),
    ]);
    let message = parse_message(&raw_message);

    assert_eq!(attachment_bytes(&raw_message, 0).unwrap(), b"bare");
    assert_eq!(message.attachments[0].filename, None);
    assert_eq!(message.attachments[0].content_type, "text/plain");
    assert_eq!(attachment_bytes(&raw_message, 1).unwrap(), b"single");
    assert_eq!(message.attachments[1].filename.as_deref(), Some("café.txt"));
    assert_eq!(attachment_bytes(&raw_message, 2).unwrap(), b"continued");
    assert_eq!(
        message.attachments[2].filename.as_deref(),
        Some("long name.txt")
    );
}

/// RFC 2231 supports language-tagged charsets and both encoded and plain parameter continuations.
#[test]
fn filenames_decode_languages_legacy_charsets_and_mixed_continuations() {
    let raw_message = multipart_message(&[
        (
            "Content-Disposition: attachment; filename*=ISO-8859-1'fr'caf%E9.txt",
            b"one",
        ),
        (
            "Content-Disposition: attachment; filename*0=\"long \"; filename*1=\"name.txt\"",
            b"two",
        ),
        (
            "Content-Disposition: attachment; filename*0*=ISO-8859-1'fr'caf%E9; filename*1=\".txt\"",
            b"three",
        ),
    ]);
    let message = parse_message(&raw_message);
    let filenames = message
        .attachments
        .iter()
        .map(|attachment| attachment.filename.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        filenames,
        [Some("café.txt"), Some("long name.txt"), Some("café.txt")]
    );
    let header_message = RawMessage::new(b"Subject: =?ISO-8859-1*fr?Q?caf=E9?=\r\n\r\nbody")
        .parse()
        .unwrap();
    assert_eq!(header_message.facts.subject, "café");
}

/// Decode nested message attachments whether their enclosing body is encoded or raw.
#[test]
fn handles_nested_rfc822_and_global_messages_in_encoded_and_raw_contexts() {
    let nested_rfc822 = b"From: nested@example.test\r\nSubject: nested\r\n\r\nbody";
    let nested_global = b"From: global@example.test\r\nSubject: global\r\n\r\nbody";
    let raw_message = multipart_message(&[
        (
            "Content-Type: message/rfc822\r\nContent-Disposition: attachment",
            nested_rfc822,
        ),
        (
            "Content-Type: message/rfc822\r\nContent-Disposition: attachment\r\nContent-Transfer-Encoding: base64",
            b"RnJvbTogbmVzdGVkQGV4YW1wbGUudGVzdA0KU3ViamVjdDogbmVzdGVkDQoNCmJvZHk=",
        ),
        (
            "Content-Type: message/global\r\nContent-Disposition: attachment",
            nested_global,
        ),
        (
            "Content-Type: message/global\r\nContent-Disposition: attachment\r\nContent-Transfer-Encoding: quoted-printable",
            b"From: global@example.test=0D=0ASubject: global=0D=0A=0D=0Abody",
        ),
    ]);
    let expected_bytes: [&[u8]; 4] = [nested_rfc822, nested_rfc822, nested_global, nested_global];
    let message = parse_message(&raw_message);

    assert_eq!(message.attachments.len(), expected_bytes.len());
    for (index, expected) in expected_bytes.iter().enumerate() {
        assert_eq!(attachment_bytes(&raw_message, index).unwrap(), *expected);
        assert_eq!(message.attachments[index].size, expected.len() as u64);
        assert_eq!(
            message.attachments[index].content_hash,
            expected_hash(expected)
        );
    }
}

/// Classify a related inline image as an attachment and hash its decoded content.
#[test]
fn classifies_related_inline_images_as_attachments_with_decoded_hashes() {
    let raw_message = b"From: sender@example.test\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=related; type=\"text/html\"\r\n\r\n--related\r\nContent-Type: text/html\r\n\r\n<img src=\"cid:image\">\r\n--related\r\nContent-Type: image/png\r\nContent-ID: <image>\r\nContent-Transfer-Encoding: base64\r\n\r\nAP9BQg==\r\n--related--\r\n";
    let expected = b"\x00\xffAB";
    let message = parse_message(raw_message);

    assert_eq!(message.attachments.len(), 1);
    assert_eq!(message.attachments[0].content_type, "image/png");
    assert_eq!(attachment_bytes(raw_message, 0).unwrap(), expected);
    assert_eq!(message.attachments[0].size, expected.len() as u64);
    assert_eq!(message.attachments[0].content_hash, expected_hash(expected));
}

/// Recover malformed transfer encodings as the raw body bytes, matching message parsing.
#[test]
fn malformed_base64_uses_the_parser_raw_body_recovery() {
    let raw_message = multipart_message(&[(
        "Content-Type: application/octet-stream\r\nContent-Disposition: attachment\r\nContent-Transfer-Encoding: base64",
        b"Y2Fm$Q==",
    )]);
    let expected = b"Y2Fm$Q==";
    let message = parse_message(&raw_message);

    assert_eq!(attachment_bytes(&raw_message, 0).unwrap(), expected);
    assert_eq!(message.attachments[0].size, expected.len() as u64);
    assert_eq!(message.attachments[0].content_hash, expected_hash(expected));
}

/// Report a missing attachment index using the dedicated error variant.
#[test]
fn missing_attachment_index_returns_attachment_not_found() {
    let raw_message = multipart_message(&[(
        "Content-Type: application/octet-stream\r\nContent-Disposition: attachment",
        b"only",
    )]);

    assert_eq!(
        attachment_bytes(&raw_message, 1),
        Err(MimeError::AttachmentNotFound { index: 1 })
    );
}
