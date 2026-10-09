//! Receiver requirements and byte fidelity at MIME recovery boundaries.
use sandpost_mime::{RawMessage, attachment_bytes};

/// Unknown charsets and transfer encodings remain opaque downloadable octets.
#[test]
fn unknown_charset_and_encoding_are_opaque_attachments() {
    for header in [
        "Content-Type: text/plain; charset=x-unknown",
        "Content-Type: text/plain\r\nContent-Transfer-Encoding: x-unknown",
    ] {
        let raw = format!("{header}\r\n\r\noriginal octets");
        let message = RawMessage::new(raw.as_bytes()).parse().unwrap();
        assert!(message.facts.text.is_empty());
        assert_eq!(
            message.attachments[0].content_type,
            "application/octet-stream"
        );
        assert_eq!(
            attachment_bytes(raw.as_bytes(), 0).unwrap(),
            b"original octets"
        );
    }
}

/// RFC 2045 permits comments and folding around a transfer encoding token.
#[test]
fn transfer_encoding_comments_and_folding_do_not_change_decoding() {
    for encoding in [
        "(before)\r\n BASE64 (after (nested))",
        "(x)\r\n BASE64",
        "\r\n BASE64 (x)",
    ] {
        let raw = format!(
            "Content-Type: text/plain\r\nContent-Transfer-Encoding: {encoding}\r\n\r\naGVsbG8="
        );
        let message = RawMessage::new(raw.as_bytes()).parse().unwrap();
        assert_eq!(message.facts.text, "hello", "{encoding}");
        assert_eq!(message.raw_message, raw.as_bytes());
    }
}

/// Related root selection ignores disposition while non-root textual resources stay downloadable.
#[test]
fn related_start_selects_root_and_keeps_text_resources_as_attachments() {
    let raw = b"Content-Type: multipart/related; boundary=rel; start=\"<root>\"; type=\"text/html\"\r\n\r\n--rel\r\nContent-Type: text/plain\r\nContent-ID: <resource>\r\n\r\nresource\r\n--rel\r\nContent-Type: text/html\r\nContent-Disposition: attachment\r\nContent-ID: <root>\r\n\r\n<b>root</b>\r\n--rel--\r\n";
    let message = RawMessage::new(raw).parse().unwrap();
    assert_eq!(message.facts.markup_body, "<b>root</b>");
    assert_eq!(message.facts.attachment_count, 1);
    assert_eq!(attachment_bytes(raw, 0).unwrap(), b"resource");
}

/// A repeated supported alternative replaces earlier representations of the same media type.
#[test]
fn alternatives_choose_the_last_supported_body_of_each_kind() {
    let raw = b"Content-Type: multipart/alternative; boundary=alt\r\n\r\n--alt\r\nContent-Type: text/plain\r\n\r\nold\r\n--alt\r\nContent-Type: text/plain\r\n\r\nnew\r\n--alt--";
    assert_eq!(RawMessage::new(raw).parse().unwrap().facts.text, "new");
}

/// Registered, vendor, and unknown media types keep their declared type and opaque body.
#[test]
fn arbitrary_media_types_preserve_opaque_payloads() {
    for media_type in [
        "application/vnd.example+json",
        "application/x-private",
        "message/global-delivery-status",
        "message/global-disposition-notification",
        "message/global-headers",
    ] {
        let raw = format!(
            "Content-Type: {media_type}\r\nContent-Disposition: attachment\r\nContent-Transfer-Encoding: base64\r\n\r\nAP9BQg=="
        );
        let message = RawMessage::new(raw.as_bytes()).parse().unwrap();
        assert_eq!(message.attachments[0].content_type, media_type);
        assert_eq!(attachment_bytes(raw.as_bytes(), 0).unwrap(), b"\0\xffAB");
    }
}

/// Delimiter repair is excluded from downloaded multipart attachment bytes and their metadata.
#[test]
fn recovery_does_not_insert_bytes_into_multipart_attachments() {
    let raw = b"Content-Type: multipart/mixed; boundary=outer\r\n\r\n--outer\r\nContent-Type: multipart/mixed; boundary=inner\r\nContent-Disposition: attachment\r\nContent-ID: <inner>\r\n\r\n--inner\r\nContent-Type: text/plain\r\n\r\npayload\r\n--outer--\r\n";
    // Multipart containers need not appear in the flat attachment list; CID resolves the entity.
    assert_eq!(
        sandpost_mime::content_reference_bytes(raw, "cid:inner").unwrap(),
        b"--inner\r\nContent-Type: text/plain\r\n\r\npayload"
    );
    assert_eq!(RawMessage::new(raw).parse().unwrap().raw_message, raw);
}

/// A deeply nested valid multipart selects its body without recursive application traversal.
#[test]
fn deeply_nested_multipart_keeps_its_body_and_raw_bytes() {
    let mut raw = String::new();
    for depth in 0..512 {
        raw.push_str(&format!(
            "Content-Type: multipart/mixed; boundary=level{depth}\r\n\r\n--level{depth}\r\n"
        ));
    }
    raw.push_str("Content-Type: text/plain\r\n\r\ndeep body");
    for depth in (0..512).rev() {
        raw.push_str(&format!("\r\n--level{depth}--"));
    }
    let message = RawMessage::new(raw.as_bytes()).parse().unwrap();
    assert_eq!(message.facts.text, "deep body");
    assert_eq!(message.raw_message, raw.as_bytes());
}

/// Enclosing boundaries also terminate incomplete multipart bodies inside embedded messages.
#[test]
fn outer_boundaries_recover_inside_embedded_messages() {
    let embedded = b"Subject: forwarded\r\nContent-Type: multipart/mixed; boundary=inner\r\nContent-Transfer-Encoding: 7bit (identity)\r\n\r\n--inner\r\nContent-Type: text/plain\r\n\r\ninside";
    let mut raw = b"Content-Type: multipart/mixed; boundary=outer\r\nContent-Transfer-Encoding: 7bit (identity)\r\n\r\n--outer\r\nContent-Type: message/rfc822\r\nContent-Transfer-Encoding: 7bit (identity)\r\n\r\n".to_vec();
    raw.extend_from_slice(embedded);
    raw.extend_from_slice(
        b"\r\n--outer\r\nContent-Type: text/plain\r\n\r\nsibling\r\n--outer--\r\n",
    );
    let message = RawMessage::new(raw.clone()).parse().unwrap();
    assert_eq!(message.facts.text, "sibling");
    assert_eq!(message.facts.attachment_count, 1);
    assert_eq!(attachment_bytes(&raw, 0).unwrap(), embedded);
    assert_eq!(message.raw_message, raw);
}

/// Permissive LF-only recovery excludes the enclosing delimiter's newline from downloaded data.
#[test]
fn recovery_preserves_lf_only_multipart_attachment_octets() {
    let raw = b"Content-Type: multipart/mixed; boundary=outer\n\n--outer\nContent-Type: multipart/mixed; boundary=inner\nContent-ID: <inner>\n\n--inner\nContent-Type: text/plain\n\npayload\n--outer--\n";
    assert_eq!(
        sandpost_mime::content_reference_bytes(raw, "cid:inner").unwrap(),
        b"--inner\nContent-Type: text/plain\n\npayload"
    );
    assert_eq!(RawMessage::new(raw).parse().unwrap().raw_message, raw);
}
