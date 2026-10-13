//! BODY and BODYSTRUCTURE (RFC 3501 section 7.4.2) from the parsed entity tree.
use crate::message::{
    envelope::write_envelope,
    headers::{StructuredValue, metadata_value, split_unquoted},
    structure::{Children, Entity},
};
use crate::protocol::response::Output;

/// Write the body structure of an entity; `extensible` selects BODYSTRUCTURE over BODY.
pub fn write_body_structure(
    output: &mut Output,
    message: &[u8],
    entity: &Entity,
    extensible: bool,
) {
    output.raw("(");
    match &entity.children {
        Children::Multipart(children) => {
            for child in children {
                write_body_structure(output, message, child, extensible);
            }
            output.raw(" ").string(entity.media_subtype.as_bytes());
            if extensible {
                output.raw(" ");
                write_parameters(output, &entity.parameters);
                output.raw(" ");
                write_extension_tail(output, message, entity);
            }
        }
        children => {
            output.string(entity.media_type.as_bytes()).raw(" ");
            output.string(entity.media_subtype.as_bytes()).raw(" ");
            write_parameters(output, &entity.parameters);
            output.raw(" ");
            output.nstring(metadata_value(message, &entity.fields, "content-id").as_deref());
            output.raw(" ");
            output
                .nstring(metadata_value(message, &entity.fields, "content-description").as_deref());
            output.raw(" ");
            output
                .string(entity.transfer_encoding.to_ascii_uppercase().as_bytes())
                .raw(" ");
            output.number(entity.body.len());
            let body = &message[entity.body.clone()];
            match children {
                Children::Message(inner)
                    if entity.media_type == "message" && entity.media_subtype == "rfc822" =>
                {
                    output.raw(" ");
                    write_envelope(output, message, &inner.fields);
                    output.raw(" ");
                    write_body_structure(output, message, inner, extensible);
                    output.raw(" ").number(line_count(body));
                }
                _ if entity.media_type == "text" => {
                    output.raw(" ").number(line_count(body));
                }
                _ => {}
            }
            if extensible {
                output.raw(" ");
                output.nstring(metadata_value(message, &entity.fields, "content-md5").as_deref());
                output.raw(" ");
                write_extension_tail(output, message, entity);
            }
        }
    }
    output.raw(")");
}

/// Write `body-fld-param`: a parenthesized list of name/value strings, or NIL.
fn write_parameters(output: &mut Output, parameters: &[(String, Vec<u8>)]) {
    if parameters.is_empty() {
        output.raw("NIL");
        return;
    }
    output.raw("(");
    for (index, (name, value)) in parameters.iter().enumerate() {
        if index > 0 {
            output.raw(" ");
        }
        output.string(name.as_bytes()).raw(" ").string(value);
    }
    output.raw(")");
}

/// Write the shared extension tail: disposition, language, and location.
fn write_extension_tail(output: &mut Output, message: &[u8], entity: &Entity) {
    match metadata_value(message, &entity.fields, "content-disposition")
        .map(|raw| StructuredValue::parse(&raw))
        .filter(|disposition| !disposition.value.is_empty())
    {
        Some(disposition) => {
            output
                .raw("(")
                .string(disposition.value.as_bytes())
                .raw(" ");
            write_parameters(output, &disposition.parameters);
            output.raw(")");
        }
        None => {
            output.raw("NIL");
        }
    }
    output.raw(" ");
    let languages: Vec<Vec<u8>> = metadata_value(message, &entity.fields, "content-language")
        .map(|raw| {
            split_unquoted(&raw, b',')
                .into_iter()
                .map(|language| language.trim_ascii().to_vec())
                .filter(|language| !language.is_empty())
                .collect()
        })
        .unwrap_or_default();
    match languages.as_slice() {
        [] => {
            output.raw("NIL");
        }
        [language] => {
            output.string(language);
        }
        languages => {
            output.raw("(");
            for (index, language) in languages.iter().enumerate() {
                if index > 0 {
                    output.raw(" ");
                }
                output.string(language);
            }
            output.raw(")");
        }
    }
    output.raw(" ");
    output.nstring(metadata_value(message, &entity.fields, "content-location").as_deref());
}

/// Count the lines of a body: line feeds, plus a final unterminated line.
fn line_count(body: &[u8]) -> usize {
    let line_feeds = body.iter().filter(|byte| **byte == b'\n').count();
    if body.is_empty() || body.ends_with(b"\n") {
        line_feeds
    } else {
        line_feeds + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::structure::parse_message;

    /// Render a message's BODY or BODYSTRUCTURE as text.
    fn render(message: &[u8], extensible: bool) -> String {
        let root = parse_message(message);
        let mut output = Output::new();
        write_body_structure(&mut output, message, &root, extensible);
        String::from_utf8(output.as_bytes().to_vec()).unwrap()
    }

    /// A plain message reports default type, parameters, encoding, size, and lines.
    #[test]
    fn single_text_part() {
        let message = b"Subject: x\r\n\r\nline one\r\nline two\r\n";
        assert_eq!(
            render(message, false),
            "(\"text\" \"plain\" (\"charset\" \"us-ascii\") NIL NIL \"7BIT\" 20 2)"
        );
        assert_eq!(
            render(message, true),
            "(\"text\" \"plain\" (\"charset\" \"us-ascii\") NIL NIL \"7BIT\" 20 2 NIL NIL NIL NIL)"
        );
    }

    /// Multiparts list their parts then subtype, with extension data for BODYSTRUCTURE.
    #[test]
    fn multipart_with_attachment() {
        let message = b"Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\nContent-Language: en, it\r\n\r\n<p>hi</p>\r\n--b\r\nContent-Type: application/pdf; name=\"r.pdf\"\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: attachment; filename=\"r.pdf\"\r\nContent-ID: <c1>\r\n\r\nJVBERg==\r\n--b--\r\n";
        assert_eq!(
            render(message, true),
            "((\"text\" \"html\" (\"charset\" \"utf-8\") NIL NIL \"QUOTED-PRINTABLE\" 9 1 NIL NIL (\"en\" \"it\") NIL)(\"application\" \"pdf\" (\"name\" \"r.pdf\") \"<c1>\" NIL \"BASE64\" 8 NIL (\"attachment\" (\"filename\" \"r.pdf\")) NIL NIL) \"mixed\" (\"boundary\" \"b\") NIL NIL NIL)"
        );
        assert_eq!(
            render(message, false),
            "((\"text\" \"html\" (\"charset\" \"utf-8\") NIL NIL \"QUOTED-PRINTABLE\" 9 1)(\"application\" \"pdf\" (\"name\" \"r.pdf\") \"<c1>\" NIL \"BASE64\" 8) \"mixed\")"
        );
    }

    /// Encapsulated messages carry an envelope, nested structure, and line count.
    #[test]
    fn encapsulated_message() {
        let message = b"Content-Type: message/rfc822\r\n\r\nSubject: inner\r\n\r\nhello\r\n";
        assert_eq!(
            render(message, false),
            "(\"message\" \"rfc822\" NIL NIL NIL \"7BIT\" 25 (NIL \"inner\" NIL NIL NIL NIL NIL NIL NIL NIL) (\"text\" \"plain\" (\"charset\" \"us-ascii\") NIL NIL \"7BIT\" 7 1) 3)"
        );
    }

    /// Line counting includes a final line without a line feed.
    #[test]
    fn line_counts() {
        assert_eq!(line_count(b""), 0);
        assert_eq!(line_count(b"a"), 1);
        assert_eq!(line_count(b"a\r\nb"), 2);
        assert_eq!(line_count(b"a\r\n"), 1);
    }

    /// BODY/BODYSTRUCTURE use normalized MIME type and encoding tokens, including text lines.
    #[test]
    fn structured_mime_tokens_are_normalized() {
        let raw = b"Content-Type: text (comment) / plain\r\nContent-Transfer-Encoding: base64 (comment)\r\n\r\naGVsbG8=\r\n";
        assert_eq!(
            render(raw, false),
            "(\"text\" \"plain\" NIL NIL NIL \"BASE64\" 10 1)"
        );
        assert_eq!(
            render(raw, true),
            "(\"text\" \"plain\" NIL NIL NIL \"BASE64\" 10 1 NIL NIL NIL NIL)"
        );
    }

    /// At a depth cutoff the body is a complete opaque structure rather than an incomplete message.
    #[test]
    fn depth_overflow_renders_valid_opaque_structure() {
        let raw = format!(
            "{}Subject: x\r\n\r\nx\r\n",
            "Content-Type: message/rfc822\r\n\r\n".repeat(33)
        );
        let root = parse_message(raw.as_bytes());
        let mut cutoff = &root;
        while let Children::Message(inner) = &cutoff.children {
            cutoff = inner;
        }
        let mut output = Output::new();
        write_body_structure(&mut output, raw.as_bytes(), cutoff, false);
        assert_eq!(
            output.as_bytes(),
            b"(\"application\" \"octet-stream\" NIL NIL NIL \"7BIT\" 17)"
        );
        for extensible in [false, true] {
            use imap_codec::{ResponseCodec, decode::Decoder};
            let response = format!(
                "* 1 FETCH (BODYSTRUCTURE {})\r\n",
                render(raw.as_bytes(), extensible)
            );
            let (rest, _) = ResponseCodec::default()
                .decode(response.as_bytes())
                .unwrap();
            assert!(rest.is_empty());
        }
    }

    /// Deep and wide multiparts have opaque, byte-sized fallback structures with no fake children.
    #[test]
    fn multipart_overflow_renders_valid_opaque_structure() {
        let mut deep = b"Content-Type: text/plain\r\n\r\nx\r\n".to_vec();
        for index in 0..33 {
            let mut outer =
                format!("Content-Type: multipart/mixed; boundary=b{index}\r\n\r\n--b{index}\r\n")
                    .into_bytes();
            outer.extend_from_slice(&deep);
            outer.extend_from_slice(format!("\r\n--b{index}--\r\n").as_bytes());
            deep = outer;
        }
        let wide = format!(
            "Content-Type: multipart/mixed; boundary=b\r\n\r\n{}--b--\r\n",
            "--b\r\n\r\nx\r\n".repeat(3000)
        )
        .into_bytes();
        for raw in [&deep, &wide] {
            let root = parse_message(raw);
            let mut cutoff = &root;
            while let Children::Multipart(children) = &cutoff.children {
                assert_eq!(children.len(), 1);
                cutoff = &children[0];
            }
            assert_eq!(cutoff.media_type, "application");
            let mut output = Output::new();
            write_body_structure(&mut output, raw, cutoff, false);
            assert_eq!(
                String::from_utf8_lossy(output.as_bytes()),
                format!(
                    "(\"application\" \"octet-stream\" NIL NIL NIL \"7BIT\" {})",
                    cutoff.body.len()
                )
            );
            use imap_codec::{ResponseCodec, decode::Decoder};
            // imap-codec limits multipart nesting more tightly than our documented depth 32.
            // Validate the complete fallback separately from that codec's recursion limit.
            output.clear();
            output.raw("* 1 FETCH (BODYSTRUCTURE ");
            write_body_structure(&mut output, raw, cutoff, true);
            output.raw(")\r\n");
            let (rest, _) = ResponseCodec::default().decode(output.as_bytes()).unwrap();
            assert!(rest.is_empty());
        }
    }

    /// Raw NUL metadata is omitted, while encoded words and extended parameters stay encoded ASCII.
    #[test]
    fn body_structure_metadata_is_nul_free() {
        let raw = b"Content-Type: application/octet-stream; name=\"a\0b\"; filename*=utf-8''%00\r\nContent-Transfer-Encoding: 7bit\0\r\nContent-ID: a\0b\r\nContent-Description: =?utf-8?B?AA==?=\r\nContent-Disposition: attachment; filename=\"a\0b\"\r\nContent-Language: en\0\r\nContent-Location: a\0b\r\n\r\nbody";
        let structure = render(raw, true);
        assert!(!structure.as_bytes().contains(&0));
        assert!(structure.contains("\"filename*\" \"utf-8''%00\""));
        assert!(structure.contains("=?utf-8?B?AA==?="));
        assert!(structure.contains("\"7BIT\""));
        assert!(!structure.contains("\"name\""));
        use imap_codec::{ResponseCodec, decode::Decoder};
        let response = format!("* 1 FETCH (BODYSTRUCTURE {structure})\r\n");
        let (rest, _) = ResponseCodec::default()
            .decode(response.as_bytes())
            .unwrap();
        assert!(rest.is_empty());
    }
}
