//! RFC 2392 lookup of inline resources and message-scoped references.
use sandpost_mime::{MimeError, content_reference_bytes};

/// A related message with identifiers whose URI forms require percent decoding.
fn related_message() -> &'static [u8] {
    b"Message-ID: <Message/ID@example.test>\r\nContent-Type: multipart/related; boundary=ref; type=\"text/html\"\r\n\r\n--ref\r\nContent-Type: text/html\r\nContent-ID: <Root@example.test>\r\n\r\n<img src=\"cid:Image%2F%2Bid@example.test\">\r\n--ref\r\nContent-Type: image/png\r\nContent-ID: <Image/+id@example.test>\r\nContent-Transfer-Encoding: base64\r\n\r\nAP9BQg==\r\n--ref--\r\n"
}

/// cid: references preserve case and plus signs and decode percent-escaped identifier characters.
#[test]
fn cid_urls_resolve_transfer_decoded_inline_octets() {
    assert_eq!(
        content_reference_bytes(related_message(), "cid:Image%2F%2Bid@example.test").unwrap(),
        b"\x00\xffAB"
    );
    assert_eq!(
        content_reference_bytes(related_message(), "CID:Image/+id@example.test").unwrap(),
        b"\x00\xffAB"
    );
    assert!(matches!(
        content_reference_bytes(related_message(), "cid:image/+id@example.test"),
        Err(MimeError::ContentReferenceNotFound(_))
    ));
}

/// mid: long forms require the matching Message-ID and can also return the entire captured message.
#[test]
fn mid_urls_resolve_only_within_the_matching_message() {
    let raw = related_message();
    assert_eq!(
        content_reference_bytes(
            raw,
            "mid:Message%2FID@example.test/Image%2F%2Bid@example.test"
        )
        .unwrap(),
        b"\x00\xffAB"
    );
    assert_eq!(
        content_reference_bytes(raw, "mid:Message%2FID@example.test").unwrap(),
        raw
    );
    assert!(matches!(
        content_reference_bytes(raw, "mid:other@example.test/Image%2F%2Bid@example.test"),
        Err(MimeError::ContentReferenceNotFound(_))
    ));
}

/// Invalid URI schemes and escapes fail before any external resolution could occur.
#[test]
fn content_references_reject_invalid_escapes_and_missing_parts() {
    for reference in [
        "cid:",
        "cid:bad%",
        "cid:bad%XZ",
        "cid:bad%0Aid",
        "cid:%3Cid%3E",
        "https://example.test/image",
        "mid:",
        "mid:Message%2FID@example.test/",
    ] {
        assert!(
            matches!(
                content_reference_bytes(related_message(), reference),
                Err(MimeError::InvalidContentReference(_))
            ),
            "{reference}"
        );
    }
    assert!(matches!(
        content_reference_bytes(related_message(), "cid:missing@example.test"),
        Err(MimeError::ContentReferenceNotFound(_))
    ));
}

/// Duplicate Content-IDs in alternatives resolve to the final available representation.
#[test]
fn duplicate_content_ids_follow_alternative_preference_order() {
    let raw = b"Content-Type: multipart/alternative; boundary=alt\r\n\r\n--alt\r\nContent-Type: text/plain\r\nContent-ID: <shared@example.test>\r\n\r\nfirst\r\n--alt\r\nContent-Type: text/html\r\nContent-ID: <shared@example.test>\r\n\r\n<strong>last</strong>\r\n--alt--\r\n";
    assert_eq!(
        content_reference_bytes(raw, "cid:shared@example.test").unwrap(),
        b"<strong>last</strong>"
    );
}
