use sandpost_mime::{RawMessage, attachment_bytes};
use sha2::{Digest, Sha256};

/// Return the lowercase SHA-256 digest of expected attachment octets.
fn expected_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Recover an inner multipart that is missing its close before an outer opening delimiter.
#[test]
fn recovers_one_missing_inner_close_before_outer_opening_delimiter() {
    let raw_message = b"From: sender@example.test\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=outer\r\n\r\n--outer\r\nContent-Type: multipart/alternative; boundary=inner\r\n\r\n--inner\r\nContent-Type: text/plain\r\n\r\ninner plain text\r\n--outer \t\r\nContent-Type: text/plain\r\n\r\nouter plain text\r\n--outer--\r\n";

    let message = RawMessage::new(raw_message.as_slice()).parse().unwrap();

    assert_eq!(message.facts.text, "inner plain text\nouter plain text");
    assert_eq!(message.raw_message, raw_message);
}

/// Recover two nested multipart closes before the enclosing multipart close.
#[test]
fn recovers_two_missing_nested_closes_before_outer_closing_delimiter() {
    let raw_message = b"From: sender@example.test\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=outer\r\n\r\n--outer\r\nContent-Type: multipart/related; boundary=middle\r\n\r\n--middle\r\nContent-Type: multipart/alternative; boundary=inner\r\n\r\n--inner\r\nContent-Type: text/plain\r\n\r\nrecovered through two levels\r\n--outer--\t\r\n";

    let message = RawMessage::new(raw_message.as_slice()).parse().unwrap();

    assert_eq!(message.facts.text, "recovered through two levels");
    assert_eq!(message.raw_message, raw_message);
}

/// Preserve attachment bytes and hashes when a missing inner close precedes a parent boundary.
#[test]
fn recovers_nested_attachment_without_changing_original_or_decoded_octets() {
    let attachment_content = b"\x00\xffpayload\rwith-bytes";
    let raw_message = b"From: sender@example.test\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=outer\r\n\r\n--outer\r\nContent-Type: multipart/mixed; boundary=inner\r\n\r\n--inner\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=payload.bin\r\nContent-Transfer-Encoding: base64\r\n\r\nAP9wYXlsb2FkDXdpdGgtYnl0ZXM=\r\n--outer--\r\n";
    let message = RawMessage::new(raw_message.as_slice()).parse().unwrap();

    assert_eq!(message.raw_message, raw_message);
    assert_eq!(message.attachments.len(), 1);
    assert_eq!(message.attachments[0].size, attachment_content.len() as u64);
    assert_eq!(
        message.attachments[0].content_hash,
        expected_hash(attachment_content)
    );
    assert_eq!(
        attachment_bytes(raw_message, 0).unwrap(),
        attachment_content
    );
}

/// Keep boundary prefixes distinct and allow legal whitespace after physical delimiters.
#[test]
fn distinguishes_prefix_collisions_and_accepts_delimiter_trailing_whitespace() {
    let raw_message = b"From: sender@example.test\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=outer\r\n\r\n--outer-prefix\r\nthis is not an outer delimiter\r\n--outer\t \r\nContent-Type: multipart/alternative; boundary=inner\r\n\r\n--inner\r\nContent-Type: text/plain\r\n\r\nprefix-safe body\r\n--outer\t\r\nContent-Type: text/plain\r\n\r\nfollowing sibling\r\n--outer-- \t\r\n";

    let message = RawMessage::new(raw_message.as_slice()).parse().unwrap();

    assert_eq!(message.facts.text, "prefix-safe body\nfollowing sibling");
    assert_eq!(message.raw_message, raw_message);
}

/// Ignore Content-Type-looking body text when discovering nested multipart boundaries.
#[test]
fn does_not_discover_multipart_headers_inside_a_text_body() {
    let raw_message = b"From: sender@example.test\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=outer\r\n\r\n--outer\r\nContent-Type: multipart/alternative; boundary=inner\r\n\r\n--inner\r\nContent-Type: text/plain\r\n\r\nContent-Type: multipart/mixed; boundary=body-only\r\n--body-only\r\nthis remains ordinary text\r\n--outer--\r\n";

    let message = RawMessage::new(raw_message.as_slice()).parse().unwrap();

    assert!(message.facts.text.contains("Content-Type: multipart/mixed"));
    assert!(message.facts.text.contains("this remains ordinary text"));
    assert_eq!(message.raw_message, raw_message);
}
