use sandpost_mime::RawMessage;

/// Parse one captured message and return its public normalized message representation.
fn parse(raw_message: impl Into<Vec<u8>>) -> sandpost_core::Message {
    RawMessage::new(raw_message).received_at(1).parse().unwrap()
}

/// RFC 2045 defaults an omitted media type and transfer encoding to plain US-ASCII and 7bit.
#[test]
fn rfc2045_defaults_to_plain_ascii_and_seven_bit_identity() {
    let raw_message = b"MIME-Version: 1.0\r\n\r\ndefault body";
    let message = parse(raw_message.to_vec());

    assert_eq!(message.facts.text, "default body");
    assert_eq!(message.raw_message, raw_message);
}

/// RFC 2045 defines 7bit, 8bit, and binary as identity transfer encodings.
#[test]
fn rfc2045_identity_transfer_encodings_preserve_captured_octets() {
    for (encoding, body) in [
        ("7bit", b"seven bit".as_slice()),
        ("8bit", b"caf\xc3\xa9".as_slice()),
        ("binary", b"binary\0octets\xff".as_slice()),
    ] {
        let mut raw_message = format!(
            "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: {encoding}\r\n\r\n"
        )
        .into_bytes();
        raw_message.extend_from_slice(body);
        let message = parse(raw_message.clone());

        assert_eq!(message.raw_message, raw_message, "{encoding}");
        assert_eq!(message.facts.attachment_count, 0, "{encoding}");
        if encoding == "8bit" {
            assert_eq!(message.facts.text, "café");
        }
    }
}

/// RFC 2045 Base64 and quoted-printable fixtures decode their encoded octets as text.
#[test]
fn rfc2045_decodes_base64_and_quoted_printable_text() {
    let base64_message = parse(
        b"Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8gY2Fmw6k="
            .to_vec(),
    );
    let quoted_printable_message = parse(
        b"Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nhello=20caf=C3=A9"
            .to_vec(),
    );

    assert_eq!(base64_message.facts.text, "hello café");
    assert_eq!(quoted_printable_message.facts.text, "hello café");
    assert!(base64_message.raw_message.ends_with(b"aGVsbG8gY2Fmw6k="));
    assert!(
        quoted_printable_message
            .raw_message
            .ends_with(b"hello=20caf=C3=A9")
    );
}

/// RFC 2045 charset labels convert ISO-8859-1 and Windows-1252 bytes into Unicode text.
#[test]
fn rfc2045_converts_declared_legacy_charsets_without_changing_raw_bytes() {
    for (charset, body, expected) in [
        ("ISO-8859-1", b"caf\xe9".as_slice(), "café"),
        ("windows-1252", b"price \x80".as_slice(), "price €"),
    ] {
        let mut raw_message =
            format!("Content-Type: text/plain; charset={charset}\r\n\r\n").into_bytes();
        raw_message.extend_from_slice(body);
        let message = parse(raw_message.clone());

        assert_eq!(message.facts.text, expected, "{charset}");
        assert_eq!(message.raw_message, raw_message, "{charset}");
    }
}

/// RFC 2045 requires an unknown transfer encoding to be treated as opaque octet-stream data.
#[test]
fn rfc2045_unknown_transfer_encoding_recovers_without_decoding_as_text() {
    let raw_message = b"Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: x-private\r\n\r\nvisible-looking body";
    let message = parse(raw_message.to_vec());

    assert_eq!(message.facts.text, "");
    assert_eq!(message.raw_message, raw_message);
}

/// RFC 2046 nested mixed multiparts ignore preamble and epilogue text as message bodies.
#[test]
fn rfc2046_mixed_multipart_nests_and_discards_preamble_and_epilogue() {
    let message = parse(
        b"Content-Type: multipart/mixed; boundary=outer\r\n\r\nouter preamble\r\n--outer\r\nContent-Type: text/plain\r\n\r\nfirst body\r\n--outer\r\nContent-Type: multipart/mixed; boundary=inner\r\n\r\ninner preamble\r\n--inner\r\nContent-Type: text/plain\r\n\r\nnested body\r\n--inner--\r\ninner epilogue\r\n--outer--\r\nouter epilogue"
            .to_vec(),
    );

    assert_eq!(message.facts.text, "first body\nnested body");
    assert!(!message.facts.text.contains("preamble"));
    assert!(!message.facts.text.contains("epilogue"));
}

/// RFC 2046 multipart/alternative carries alternative representations of the same content.
#[test]
fn rfc2046_alternative_multipart_extracts_text_and_html_representations() {
    let message = parse(
        b"Content-Type: multipart/alternative; boundary=alt\r\n\r\n--alt\r\nContent-Type: text/plain\r\n\r\nplain choice\r\n--alt\r\nContent-Type: text/html\r\n\r\n<strong>html choice</strong>\r\n--alt--"
            .to_vec(),
    );

    assert_eq!(message.facts.text, "plain choice");
    assert_eq!(message.facts.markup_body, "<strong>html choice</strong>");
}

/// RFC 2046 multipart/digest uses message/rfc822 as the default type for its parts.
#[test]
fn rfc2046_digest_defaults_each_part_to_an_embedded_message() {
    let message = parse(
        b"Content-Type: multipart/digest; boundary=digest\r\n\r\n--digest\r\n\r\nSubject: first digest item\r\nContent-Type: text/plain\r\n\r\nfirst digest body\r\n--digest\r\n\r\nSubject: second digest item\r\nContent-Type: text/plain\r\n\r\nsecond digest body\r\n--digest--"
            .to_vec(),
    );

    assert_eq!(message.facts.attachment_count, 2);
    assert!(
        message
            .attachments
            .iter()
            .all(|part| part.content_type == "message/rfc822")
    );
    assert!(
        sandpost_mime::attachment_bytes(&message.raw_message, 0)
            .unwrap()
            .ends_with(b"first digest body")
    );
    assert!(
        sandpost_mime::attachment_bytes(&message.raw_message, 1)
            .unwrap()
            .ends_with(b"second digest body")
    );
}

/// RFC 2046 multipart/parallel groups distinct body parts within one multipart entity.
#[test]
fn rfc2046_parallel_multipart_keeps_text_parts_in_the_message_facts() {
    let message = parse(
        b"Content-Type: multipart/parallel; boundary=parallel\r\n\r\n--parallel\r\nContent-Type: text/plain\r\n\r\nparallel first\r\n--parallel\r\nContent-Type: text/plain\r\n\r\nparallel second\r\n--parallel--"
            .to_vec(),
    );

    assert_eq!(message.facts.text, "parallel first\nparallel second");
}

/// RFC 2046 requires an unrecognized multipart subtype to retain multipart structure.
#[test]
fn rfc2046_unknown_multipart_subtype_still_parses_its_parts() {
    let message = parse(
        b"Content-Type: multipart/x-future; boundary=future\r\n\r\n--future\r\nContent-Type: text/plain\r\n\r\nfuture subtype body\r\n--future--"
            .to_vec(),
    );

    assert_eq!(message.facts.text, "future subtype body");
}

/// A missing inner close delimiter must not consume the next well-formed outer part.
#[test]
fn rfc2046_nested_multipart_recovers_at_the_outer_boundary() {
    let message = parse(
        b"Content-Type: multipart/mixed; boundary=outer\r\n\r\n--outer\r\nContent-Type: multipart/mixed; boundary=inner\r\n\r\n--inner\r\nContent-Type: text/plain\r\n\r\ninner text\r\n--outer\r\nContent-Type: text/plain\r\n\r\nouter sibling\r\n--outer--"
            .to_vec(),
    );

    assert_eq!(message.facts.text, "inner text\nouter sibling");
}

/// RFC 2046 message/rfc822 embeds a complete message whose body remains parseable.
#[test]
fn rfc2046_embedded_message_rfc822_preserves_and_extracts_inner_body() {
    let raw_message = b"Content-Type: multipart/mixed; boundary=outer\r\n\r\n--outer\r\nContent-Type: message/rfc822\r\nContent-Disposition: attachment; filename=forwarded.eml\r\n\r\nFrom: sender@example.test\r\nSubject: forwarded\r\nContent-Type: text/plain\r\n\r\nembedded body\r\n--outer--";
    let message = parse(raw_message.to_vec());

    assert_eq!(message.raw_message, raw_message);
    assert_eq!(message.facts.attachment_count, 1);
    assert!(
        sandpost_mime::attachment_bytes(raw_message, 0)
            .unwrap()
            .ends_with(b"embedded body")
    );
}

/// RFC 2045 and RFC 2046 media type and encoding tokens compare without regard to case.
#[test]
fn mime_type_subtype_charset_and_transfer_tokens_are_case_insensitive() {
    let message = parse(
        b"MIME-Version: 1.0\r\nContent-Type: TEXT/PLAIN; CHARSET=UTF-8\r\nContent-Transfer-Encoding: BASE64\r\n\r\naGVsbG8="
            .to_vec(),
    );

    assert_eq!(message.facts.text, "hello");
}

/// RFC 2047 B and Q encoded words decode in subjects, extension fields, and display names.
#[test]
fn rfc2047_decodes_base64_and_quoted_printable_header_words() {
    let message = parse(
        b"From: =?ISO-8859-1?Q?Andr=E9?= Example <andre@example.test>\r\nSubject: =?UTF-8?B?Y2Fmw6k=?=\r\nX-Label: =?ISO-8859-1?Q?caf=E9?=\r\n\r\nbody"
            .to_vec(),
    );

    assert_eq!(message.facts.subject, "café");
    assert_eq!(message.facts.headers["x-label"], ["café"]);
    assert_eq!(message.facts.from[0].address, "andre@example.test");
}

/// RFC 2047 adjacent encoded words omit their separating whitespace after decoding.
#[test]
fn rfc2047_joins_adjacent_and_folded_encoded_words() {
    let message = parse(
        b"Subject: =?UTF-8?Q?caf=C3=A9?=\r\n =?UTF-8?B?bmV4dA==?=\r\nX-Joined: =?UTF-8?Q?one?= \r\n =?UTF-8?B?dHdv?=\r\n\r\nbody"
            .to_vec(),
    );

    assert_eq!(message.facts.subject, "cafénext");
    assert_eq!(message.facts.headers["x-joined"], ["onetwo"]);
}

/// RFC 2049 Appendix A's nested multipart structure yields its text parts and ignores framing.
#[test]
fn rfc2049_appendix_a_complex_multipart_fixture() {
    let message = parse(
        b"MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=unique-boundary-1\r\n\r\nThis is the preamble area of a multipart message.\r\n--unique-boundary-1\r\n\r\nintroductory text\r\n--unique-boundary-1\r\nContent-Type: text/plain; charset=US-ASCII\r\n\r\nexplicit text part\r\n--unique-boundary-1\r\nContent-Type: multipart/parallel; boundary=unique-boundary-2\r\n\r\n--unique-boundary-2\r\nContent-Type: audio/basic\r\nContent-Transfer-Encoding: base64\r\n\r\nYXVkaW8=\r\n--unique-boundary-2\r\nContent-Type: image/jpeg\r\nContent-Transfer-Encoding: base64\r\n\r\naW1hZ2U=\r\n--unique-boundary-2--\r\n--unique-boundary-1\r\nContent-Type: text/enriched\r\n\r\nThis is <bold>enriched</bold>.\r\n--unique-boundary-1\r\nContent-Type: message/rfc822\r\n\r\nFrom: sender@example.test\r\nSubject: embedded\r\nContent-Type: text/plain; charset=ISO-8859-1\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nAdditional text\r\n--unique-boundary-1--\r\nAppendix A epilogue"
            .to_vec(),
    );

    assert!(message.facts.text.contains("introductory text"));
    assert!(message.facts.text.contains("explicit text part"));
    assert!(!message.facts.text.contains("preamble"));
    assert!(!message.facts.text.contains("epilogue"));
    assert_eq!(message.raw_message.last(), Some(&b'e'));
}

/// RFC 2387 selects the related root by Content-ID, while RFC 2392 identifies inline parts.
#[test]
fn rfc2387_related_root_and_rfc2392_inline_content_ids_remain_in_raw_mime() {
    let raw_message = b"Content-Type: multipart/related; boundary=related; type=\"text/html\"; start=\"<root@example.test>\"\r\n\r\n--related\r\nContent-ID: <resource@example.test>\r\nContent-Type: image/png\r\nContent-Disposition: inline\r\nContent-Transfer-Encoding: base64\r\n\r\naW1hZ2U=\r\n--related\r\nContent-ID: <root@example.test>\r\nContent-Type: text/html\r\n\r\n<html><img src=\"cid:resource@example.test\"></html>\r\n--related--";
    let message = parse(raw_message.to_vec());

    assert_eq!(
        message.facts.markup_body,
        "<html><img src=\"cid:resource@example.test\"></html>"
    );
    assert_eq!(message.raw_message, raw_message);
    assert!(
        message
            .raw_message
            .windows(b"Content-ID: <resource@example.test>".len())
            .any(|window| window == b"Content-ID: <resource@example.test>")
    );
}

/// RFC 6532 permits direct UTF-8 in header values and corresponding UTF-8 message bodies.
#[test]
fn rfc6532_preserves_utf8_headers_and_body_text() {
    let raw_message = "From: 用户 <用户@例子.测试>\r\nTo: δοκιμή@παράδειγμα.δοκιμή\r\nSubject: café 日本語\r\nX-Note: naïve Grüße\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nZażółć gęślą jaźń — 東京".as_bytes();
    let message = parse(raw_message.to_vec());

    assert_eq!(message.facts.subject, "café 日本語");
    assert_eq!(message.facts.from[0].address, "用户@例子.测试");
    assert_eq!(message.facts.to[0].address, "δοκιμή@παράδειγμα.δοκιμή");
    assert_eq!(message.facts.headers["x-note"], ["naïve Grüße"]);
    assert_eq!(message.facts.text, "Zażółć gęślą jaźń — 東京");
    assert_eq!(message.raw_message, raw_message);
}

/// RFC 6532 and RFC 6533 define message/global as an encapsulated internationalized message.
#[test]
fn rfc6533_message_global_is_captured_as_an_attachment() {
    let raw_message = "Content-Type: multipart/mixed; boundary=outer\r\n\r\n--outer\r\nContent-Type: message/global\r\nContent-Transfer-Encoding: 8bit\r\nContent-Disposition: attachment; filename=global.eml\r\n\r\nSubject: Καλημέρα\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nγειά σου\r\n--outer--".as_bytes();
    let message = parse(raw_message.to_vec());

    assert_eq!(message.facts.attachment_count, 1);
    assert_eq!(message.raw_message, raw_message);
}
