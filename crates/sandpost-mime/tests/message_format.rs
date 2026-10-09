//! RFC 5322 header-address extraction and receiver bounds through the public MIME API.
use sandpost_mime::{MimeError, RawMessage};

/// Quoted local parts remain valid both with angle brackets and as bare addr-specs in groups.
#[test]
fn quoted_address_headers_preserve_embedded_separators_and_groups() {
    for address in [
        "\"User@Tag\"@Example.test",
        "\"User Name\"@Example.test",
        "\"comma,colon:semi;\"@Example.test",
    ] {
        for from in [address.to_owned(), format!("Display <{address}>")] {
            let raw = format!(
                "From: {from}\r\nTo: Group: {address}, Other@example.test;\r\nCc: Empty:;\r\nSubject: quoted\r\n\r\nbody\r\n"
            );
            let message = RawMessage::new(raw.clone()).parse().unwrap();
            assert_eq!(message.facts.from[0].address, address.to_ascii_lowercase());
            assert_eq!(message.facts.to.len(), 2);
            assert_eq!(message.facts.to[0].address, address.to_ascii_lowercase());
            assert_eq!(message.facts.to[1].address, "other@example.test");
            assert!(message.facts.carbon_copy.is_empty());
            assert_eq!(message.raw_message, raw.as_bytes());
        }
    }
}

/// Repeated obsolete address fields and nested comments retain all usable mailbox facts in order.
#[test]
fn repeated_address_headers_and_nested_comments_keep_mailbox_order() {
    let raw = "From: (outer (inner)) \"user name\"@example.test\r\nTo: First <first@example.test>\r\nTo: second@example.test\r\nCc: Named Group: third@example.test;\r\nSubject: comments\r\n\r\nbody\r\n";
    let message = RawMessage::new(raw).parse().unwrap();
    assert_eq!(message.facts.from[0].address, "\"user name\"@example.test");
    assert_eq!(
        message
            .facts
            .to
            .iter()
            .map(|mailbox| mailbox.address.as_str())
            .collect::<Vec<_>>(),
        ["first@example.test", "second@example.test"]
    );
    assert_eq!(message.facts.carbon_copy[0].address, "third@example.test");
}

/// The exact byte allowance accepts the original message and refuses its next octet.
#[test]
fn raw_message_size_boundary_is_inclusive() {
    let raw = b"Subject: exact\r\n\r\nbody\r\n";
    let message = RawMessage::new(raw.as_slice())
        .size_limit(raw.len())
        .parse()
        .unwrap();
    assert_eq!(message.raw_message, raw);
    assert_eq!(message.facts.size, raw.len() as u64);
    assert_eq!(
        RawMessage::new(raw.as_slice())
            .size_limit(raw.len() - 1)
            .parse()
            .unwrap_err(),
        MimeError::MessageTooLarge {
            size: raw.len(),
            limit: raw.len() - 1
        }
    );
}

/// Hundreds of MIME parts are processed iteratively without losing captured data or attachment order.
#[test]
fn many_mime_parts_preserve_all_attachment_metadata() {
    let mut raw =
        String::from("Subject: many parts\r\nContent-Type: multipart/mixed; boundary=many\r\n\r\n");
    for index in 0..256 {
        raw.push_str(&format!("--many\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=file-{index}.bin\r\n\r\nx\r\n"));
    }
    raw.push_str("--many--\r\n");
    let message = RawMessage::new(raw.clone()).parse().unwrap();
    assert_eq!(message.attachments.len(), 256);
    for (index, attachment) in message.attachments.iter().enumerate() {
        assert_eq!(
            attachment.filename.as_deref(),
            Some(format!("file-{index}.bin").as_str())
        );
        assert_eq!(attachment.size, 1);
    }
    assert_eq!(message.raw_message, raw.as_bytes());
}
