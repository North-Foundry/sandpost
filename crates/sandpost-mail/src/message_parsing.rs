//! Message parsing and normalization using the existing MIME parser.
use crate::{MailError, limits::MAXIMUM_MESSAGE_SIZE};
use mail_parser::{Address, MessageParser, MimeHeaders};
use sandpost_core::{Attachment, Mailbox, Message, MessageFacts, MessageIdentifier};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Parse a MIME message and normalize its mailbox and body fields.
pub fn parse_message(
    envelope_from: Option<&str>,
    envelope_to: &[String],
    raw_message: Vec<u8>,
    received_at: i64,
) -> Result<Message, MailError> {
    if raw_message.len() > MAXIMUM_MESSAGE_SIZE {
        return Err(MailError::InvalidMessage("message exceeds 10 MiB".into()));
    }
    let parsed = MessageParser::new()
        .with_mime_headers()
        .with_address_headers()
        .with_message_ids()
        .default_header_text()
        .parse(&raw_message)
        .ok_or_else(|| MailError::InvalidMessage("no parseable headers".into()))?;

    let envelope_from = envelope_from.map(normalize_mailbox).transpose()?;
    let envelope_to = envelope_to
        .iter()
        .map(|address| normalize_mailbox(address))
        .collect::<Result<_, _>>()?;
    let from = parsed
        .from()
        .into_iter()
        .flat_map(Address::iter)
        .filter_map(|address| address.address())
        .map(normalize_mailbox)
        .collect::<Result<_, _>>()?;
    let to = parsed
        .all_to()
        .flat_map(Address::iter)
        .filter_map(|address| address.address())
        .map(normalize_mailbox)
        .collect::<Result<_, _>>()?;
    let carbon_copy = parsed
        .all_cc()
        .flat_map(Address::iter)
        .filter_map(|address| address.address())
        .map(normalize_mailbox)
        .collect::<Result<_, _>>()?;

    let mut headers = BTreeMap::<String, Vec<String>>::new();
    for (header_index, (header_name, raw_header_value)) in parsed.headers_raw().enumerate() {
        let value = parsed
            .headers()
            .get(header_index)
            .and_then(|header| header.value().as_text())
            .unwrap_or(raw_header_value);
        headers
            .entry(header_name.to_ascii_lowercase())
            .or_default()
            .push(unfold_header(value));
    }
    let text = (0..parsed.text_body_count())
        .filter_map(|body_part_index| parsed.body_text(body_part_index))
        .map(|part| part.into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    let markup_body = (0..parsed.html_body_count())
        .filter_map(|body_part_index| parsed.body_html(body_part_index))
        .map(|part| part.into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    let attachments = parsed
        .attachments()
        .map(|part| {
            let attachment_bytes = part.contents();
            let content_type = part
                .content_type()
                .map(|content_type| match content_type.subtype() {
                    Some(subtype) => format!("{}/{}", content_type.ctype(), subtype),
                    None => content_type.ctype().to_owned(),
                })
                .unwrap_or_else(|| "application/octet-stream".into());
            Attachment {
                filename: part.attachment_name().map(str::to_owned),
                content_type,
                size: attachment_bytes.len() as u64,
                content_hash: format!("{:x}", Sha256::digest(attachment_bytes)),
            }
        })
        .collect::<Vec<_>>();

    Ok(Message {
        identifier: MessageIdentifier::new(),
        facts: MessageFacts {
            envelope_from,
            envelope_to,
            from,
            to,
            carbon_copy,
            subject: parsed.subject().unwrap_or_default().to_owned(),
            text,
            markup_body,
            message_identifier: parsed.message_id().map(str::to_owned),
            received_at,
            size: raw_message.len() as u64,
            attachment_count: attachments.len() as u64,
            headers,
        },
        raw_message,
        attachments,
    })
}

/// Normalize an email mailbox and derive its lowercase domain.
pub(crate) fn normalize_mailbox(mailbox_text: &str) -> Result<Mailbox, MailError> {
    let normalized_address = mailbox_text.trim().to_ascii_lowercase();
    let (local_part, domain) = normalized_address
        .split_once('@')
        .ok_or_else(|| MailError::InvalidMessage(format!("invalid mailbox {mailbox_text:?}")))?;
    if local_part.is_empty() || domain.is_empty() || domain.contains('@') {
        return Err(MailError::InvalidMessage(format!(
            "invalid mailbox {mailbox_text:?}"
        )));
    }
    let domain = domain.to_owned();
    Ok(Mailbox {
        address: normalized_address,
        domain,
    })
}

/// Replace folded header line breaks with spaces and trim surrounding whitespace.
fn unfold_header(header_value: &str) -> String {
    let mut unfolded_value = String::with_capacity(header_value.len());
    let mut characters = header_value.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\r' && characters.peek() == Some(&'\n') {
            characters.next();
            while matches!(characters.peek(), Some(' ' | '\t')) {
                characters.next();
            }
            unfolded_value.push(' ');
        } else {
            unfolded_value.push(character);
        }
    }
    unfolded_value.trim().to_owned()
}
