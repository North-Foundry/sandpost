//! Decoded message text for SEARCH content criteria (RFC 3501 section 6.4.4).
//!
//! String criteria match a substring case-insensitively. Header criteria compare against field
//! values with RFC 2047 encoded words decoded; BODY includes text parts and encapsulated
//! message headers/bodies; TEXT also includes the top-level header with name/value separators.
//! FETCH and SEARCH share the bounded MIME tree and normalized MIME tokens. Opaque composite
//! fallbacks are excluded from body searching. Case is folded with Unicode lowercase.
use crate::message::headers::{HeaderField, unfold};
use crate::message::structure::{Children, Entity, parse_message};
use crate::protocol::datetime::CivilDate;
use mail_parser::{
    decoders::{
        base64::base64_decode, charsets::map::charset_decoder,
        quoted_printable::quoted_printable_decode,
    },
    parsers::MessageStream,
};
use std::{borrow::Cow, cell::OnceCell};

/// One message prepared for repeated SEARCH criteria, decoding lazily.
pub struct SearchableMessage<'message> {
    raw: &'message [u8],
    root: Entity,
    decoded_headers: OnceCell<Vec<(String, String)>>,
    body_texts: OnceCell<Vec<String>>,
}

impl<'message> SearchableMessage<'message> {
    /// Index the top-level header fields of a raw message.
    pub fn new(raw: &'message [u8]) -> Self {
        Self {
            raw,
            root: parse_message(raw),
            decoded_headers: OnceCell::new(),
            body_texts: OnceCell::new(),
        }
    }

    /// Every top-level field as (lowercase name, decoded lowercase value).
    fn decoded_headers(&self) -> &[(String, String)] {
        self.decoded_headers.get_or_init(|| {
            self.root
                .fields
                .iter()
                .map(|field| {
                    (
                        field.name.to_ascii_lowercase(),
                        decode_header_value(&self.raw[field.value.clone()]).to_lowercase(),
                    )
                })
                .collect()
        })
    }

    /// Decoded lowercase text parts and headers of encapsulated messages in the bounded tree.
    fn body_texts(&self) -> &[String] {
        self.body_texts.get_or_init(|| {
            let mut texts = Vec::new();
            collect_body_texts(self.raw, &self.root, &mut texts);
            texts
        })
    }

    /// Report whether any field with this name contains the string; an empty string matches any
    /// message that has the field.
    pub fn header_contains(&self, name: &str, needle: &str) -> bool {
        let name = name.to_ascii_lowercase();
        let needle = needle.to_lowercase();
        self.decoded_headers()
            .iter()
            .any(|(field, value)| *field == name && value.contains(&needle))
    }

    /// Report whether the decoded body text contains the string.
    pub fn body_contains(&self, needle: &str) -> bool {
        let needle = needle.to_lowercase();
        self.body_texts().iter().any(|text| text.contains(&needle))
    }

    /// Report whether the header or the body contains the string.
    pub fn text_contains(&self, needle: &str) -> bool {
        let lowered = needle.to_lowercase();
        self.decoded_headers()
            .iter()
            .zip(&self.root.fields)
            .any(|((_, value), field)| header_text(self.raw, field, value).contains(&lowered))
            || self.body_contains(needle)
    }

    /// The calendar date written in the Date field, disregarding time and zone.
    pub fn sent_date(&self) -> Option<CivilDate> {
        let field = self
            .root
            .fields
            .iter()
            .find(|field| field.name.eq_ignore_ascii_case("date"))?;
        let mut value = self.raw[field.value.clone()].to_vec();
        value.extend_from_slice(b"\r\n");
        let parsed = MessageStream::new(&value).parse_date();
        let date = parsed.as_datetime()?;
        CivilDate::parse(&format!(
            "{}-{}-{:04}",
            date.day,
            [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"
            ]
            .get(usize::from(date.month).checked_sub(1)?)?,
            date.year
        ))
    }
}

/// Join a decoded value to its field name and colon, retaining the original leading whitespace.
fn header_text(raw: &[u8], field: &HeaderField, value: &str) -> String {
    let whitespace: String = raw[field.value.clone()]
        .iter()
        .take_while(|byte| byte.is_ascii_whitespace())
        .filter(|byte| matches!(byte, b' ' | b'\t'))
        .map(|byte| char::from(*byte))
        .collect();
    format!("{}:{whitespace}{value}", field.name).to_lowercase()
}

/// Collect decoded text and embedded message headers without descending into opaque fallbacks.
fn collect_body_texts(raw: &[u8], entity: &Entity, texts: &mut Vec<String>) {
    match &entity.children {
        Children::Multipart(children) => {
            for child in children {
                collect_body_texts(raw, child, texts);
            }
        }
        Children::Message(inner) => {
            for field in &inner.fields {
                let value = decode_header_value(&raw[field.value.clone()]);
                texts.push(header_text(raw, field, &value));
            }
            collect_body_texts(raw, inner, texts);
        }
        Children::None if entity.media_type == "text" => {
            let body = &raw[entity.body.clone()];
            let decoded = match entity.transfer_encoding.as_str() {
                "base64" => base64_decode(body).map(Cow::Owned),
                "quoted-printable" => quoted_printable_decode(body).map(Cow::Owned),
                _ => Some(Cow::Borrowed(body)),
            };
            if let Some(decoded) = decoded {
                let decoder = entity
                    .parameters
                    .iter()
                    .find(|(name, _)| name == "charset")
                    .and_then(|(_, value)| charset_decoder(value));
                let text = decoder.map_or_else(
                    || String::from_utf8_lossy(&decoded).into_owned(),
                    |decoder| decoder(&decoded),
                );
                texts.push(text.to_lowercase());
            }
        }
        Children::None => {}
    }
}

/// Decode RFC 2047 encoded words in one folded header value.
fn decode_header_value(folded: &[u8]) -> String {
    let mut value = folded.to_vec();
    value.extend_from_slice(b"\r\n");
    match MessageStream::new(&value).parse_unstructured().into_text() {
        Some(text) => text.into_owned(),
        None => String::from_utf8_lossy(&unfold(folded)).into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MESSAGE: &[u8] = b"From: =?utf-8?q?Caf=C3=A9_Owner?= <owner@cafe.example>\r\n\
To: Dev <dev@example.test>\r\n\
Subject: Password reset\r\n\
Date: Tue, 9 Jan 2024 23:30:00 -0500\r\n\
X-App: Bravo\r\n\
Content-Type: multipart/alternative; boundary=b\r\n\
\r\n\
--b\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
UmVzZXQgeW91ciBQQVNTV09SRCBub3cu\r\n\
--b--\r\n";

    /// Header criteria decode encoded words and compare case-insensitively.
    #[test]
    fn header_criteria_decode_and_fold_case() {
        let message = SearchableMessage::new(MESSAGE);
        assert!(message.header_contains("FROM", "CAF\u{c9} owner"));
        assert!(message.header_contains("subject", "RESET"));
        assert!(message.header_contains("x-app", ""));
        assert!(!message.header_contains("x-missing", ""));
        assert!(!message.header_contains("to", "owner"));
    }

    /// BODY decodes transfer encodings; TEXT also searches the header.
    #[test]
    fn body_and_text_criteria_decode_content() {
        let message = SearchableMessage::new(MESSAGE);
        assert!(message.body_contains("your password"));
        assert!(!message.body_contains("Bravo"));
        assert!(message.text_contains("bravo"));
        assert!(message.text_contains("password now"));
    }

    /// The sent date uses the written calendar day, not the UTC conversion.
    #[test]
    fn sent_date_ignores_zone() {
        let message = SearchableMessage::new(MESSAGE);
        assert_eq!(
            message.sent_date(),
            Some(CivilDate {
                year: 2024,
                month: 1,
                day: 9
            })
        );
        assert_eq!(
            SearchableMessage::new(b"Subject: x\r\n\r\n").sent_date(),
            None
        );
    }

    /// Text searches retain header names, colons, and original leading whitespace.
    #[test]
    fn text_search_preserves_header_separators() {
        let raw = b"Subject: hello\r\nX-Tight:value\r\nX-Tab:\tthing\r\n\r\nbody";
        let message = SearchableMessage::new(raw);
        for needle in ["SUBJECT: HELLO", "x-tight:value", "x-tab:\tthing"] {
            assert!(message.text_contains(needle), "{needle}");
        }
        assert!(!message.header_contains("subject", "subject: hello"));
        assert!(!message.body_contains("subject: hello"));
    }

    /// SEARCH uses FETCH's normalized MIME tokens, transfer decoding, and charset conversion.
    #[test]
    fn body_search_normalizes_mime_and_transfer_encoding() {
        let raw = b"Content-Type: multipart (comment) / mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text / plain; charset=iso-8859-1\r\nContent-Transfer-Encoding: quoted-printable (comment)\r\nContent-Disposition: attachment\r\n\r\nCaf=E9\r\n--b\r\nContent-Type: text / html\r\nContent-Transfer-Encoding: BaSe64 (comment)\r\n\r\naGVsbG8=\r\n--b--\r\n";
        let message = SearchableMessage::new(raw);
        assert!(message.body_contains("CAFÉ"));
        assert!(message.body_contains("hello"));
        assert_eq!(message.raw, raw);
    }

    /// Nested message headers and bodies participate in BODY/TEXT, but not outer HEADER keys.
    #[test]
    fn nested_messages_are_searched() {
        let raw = b"Content-Type: multipart/digest; boundary=b\r\n\r\n--b\r\n\r\nSubject: =?utf-8?Q?inner_caf=C3=A9?=\r\nContent-Type: message (comment) / rfc822\r\n\r\nSubject: deepest\r\nContent-Type: text/plain\r\nContent-Transfer-Encoding: base64 (comment)\r\n\r\nbmVlZGxl\r\n--b--\r\n";
        let message = SearchableMessage::new(raw);
        assert!(message.body_contains("needle"));
        assert!(message.text_contains("subject: inner café"));
        assert!(message.body_contains("subject: deepest"));
        assert!(!message.header_contains("subject", "deepest"));
    }

    /// SEARCH observes the same opaque overflow policy as FETCH while keeping outer headers.
    #[test]
    fn search_obeys_composite_resource_limits() {
        let raw = format!(
            "Subject: outer\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n{}--b--\r\n",
            "--b\r\n\r\nneedle\r\n".repeat(3000)
        );
        let message = SearchableMessage::new(raw.as_bytes());
        assert!(message.text_contains("subject: outer"));
        assert!(!message.body_contains("needle"));
        assert_eq!(message.root.media_type, "application");
    }
}
