use sandpost_mime::{
    MimeError, ParsedMessage, RawMessage, attachment_bytes, content_reference_bytes,
};
use sha2::{Digest, Sha256};

/// Return the lowercase SHA-256 digest used by normalized attachment metadata.
fn lowercase_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Verify repeated reads retain attachment order and the source octets used for metadata hashes.
#[test]
fn reusable_view_preserves_order_charset_octets_and_metadata_hashes() {
    let raw_message = b"MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=parts\r\n\r\n--parts\r\nContent-Type: text/plain; charset=iso-8859-1\r\nContent-Disposition: attachment; filename=first.txt\r\nContent-Transfer-Encoding: base64\r\n\r\nY2Fm6Q==\r\n--parts\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=second.bin\r\nContent-Transfer-Encoding: base64\r\n\r\nAP9BQg==\r\n--parts--\r\n";
    let parsed = ParsedMessage::parse(raw_message).unwrap();
    let expected: [&[u8]; 2] = [b"caf\xe9", b"\x00\xffAB"];
    let metadata = RawMessage::new(raw_message.as_slice()).parse().unwrap();

    assert_eq!(parsed.raw_message(), raw_message);
    assert_eq!(parsed.attachment_count(), expected.len());
    for _ in 0..2 {
        for (index, expected_bytes) in expected.iter().enumerate() {
            assert_eq!(parsed.attachment_bytes(index).unwrap(), *expected_bytes);
            assert_eq!(
                attachment_bytes(raw_message, index).unwrap(),
                *expected_bytes
            );
            assert_eq!(
                metadata.attachments[index].size,
                expected_bytes.len() as u64
            );
            assert_eq!(
                metadata.attachments[index].content_hash,
                lowercase_hash(expected_bytes)
            );
        }
    }
    assert!(matches!(
        parsed.attachment_bytes(expected.len()),
        Err(MimeError::AttachmentNotFound { index: 2 })
    ));
}

/// Verify encoded CID and MID references, including parity with the legacy free function.
#[test]
fn reusable_view_resolves_encoded_cid_and_mid_references() {
    let raw_message = b"Message-ID: <Message/ID@example.test>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=refs\r\n\r\n--refs\r\nContent-Type: text/html\r\nContent-ID: <Root@example.test>\r\n\r\n<img src=\"cid:Image%2F%2Bid@example.test\">\r\n--refs\r\nContent-Type: image/png\r\nContent-ID: <Image/+id@example.test>\r\nContent-Transfer-Encoding: base64\r\n\r\nAP9BQg==\r\n--refs--\r\n";
    let parsed = ParsedMessage::parse(raw_message).unwrap();

    for reference in [
        "cid:Image%2F%2Bid@example.test",
        "mid:Message%2FID@example.test/Image%2F%2Bid@example.test",
    ] {
        assert_eq!(
            parsed.content_reference_bytes(reference).unwrap(),
            b"\x00\xffAB"
        );
        assert_eq!(
            content_reference_bytes(raw_message, reference).unwrap(),
            b"\x00\xffAB"
        );
    }
    assert_eq!(
        parsed
            .content_reference_bytes("mid:Message%2FID@example.test")
            .unwrap(),
        raw_message
    );
    assert!(matches!(
        parsed.content_reference_bytes("cid:bad%XZ"),
        Err(MimeError::InvalidContentReference(_))
    ));
    assert!(matches!(
        parsed.content_reference_bytes("cid:missing@example.test"),
        Err(MimeError::ContentReferenceNotFound(_))
    ));
}

/// Reuse an owned recovery view while preserving source bytes, part ranges, and decoded hashes.
#[test]
fn reusable_view_keeps_original_bytes_through_boundary_and_encoding_recovery() {
    let raw_message = b"Message-ID: <recovered@example.test>\r\nContent-Type: multipart/mixed; boundary=outer\r\n\r\n--outer\r\nContent-Type: multipart/mixed; boundary=inner\r\n\r\n--inner\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=first.bin\r\nContent-ID: <first@example.test>\r\nContent-Transfer-Encoding: base64\r\n\r\nAP9BQg==\r\n--outer\r\nContent-Type: text/plain; charset=iso-8859-1\r\nContent-Disposition: attachment; filename=second.txt\r\nContent-ID: <second@example.test>\r\nContent-Transfer-Encoding: (x)\r\n quoted-printable\r\n\r\ncaf=E9\r\n--outer--\r\n";
    let parsed = ParsedMessage::parse(raw_message).unwrap();
    let metadata = RawMessage::new(raw_message.as_slice()).parse().unwrap();
    let expected: [&[u8]; 2] = [b"\x00\xffAB", b"caf\xe9"];

    assert_eq!(parsed.raw_message(), raw_message);
    assert!(std::ptr::eq(
        parsed.raw_message().as_ptr(),
        raw_message.as_ptr()
    ));
    assert_eq!(parsed.attachment_count(), 2);
    assert_eq!(metadata.raw_message, raw_message);
    for (index, expected_bytes) in expected.iter().enumerate() {
        assert_eq!(parsed.attachment_bytes(index).unwrap(), *expected_bytes);
        assert_eq!(parsed.attachment_bytes(index).unwrap(), *expected_bytes);
        assert_eq!(
            metadata.attachments[index].size,
            expected_bytes.len() as u64
        );
        assert_eq!(
            metadata.attachments[index].content_hash,
            lowercase_hash(expected_bytes)
        );
    }
    assert_eq!(
        parsed
            .content_reference_bytes("cid:first@example.test")
            .unwrap(),
        expected[0]
    );
    assert_eq!(
        parsed
            .content_reference_bytes("cid:second@example.test")
            .unwrap(),
        expected[1]
    );
    assert_eq!(
        parsed
            .content_reference_bytes("mid:recovered@example.test")
            .unwrap(),
        raw_message
    );
}
