//! Attachment transfer decoding shared by downloads, decoded sizes and content hashes.
use crate::{MimeError, ParsedMessage, parsing::CapturedMessage};
use mail_parser::{Encoding, MimeHeaders, PartType, parsers::MessageStream};
use sandpost_core::Attachment;
use sha2::{Digest, Sha256};

/// Return an attachment's original octets after MIME transfer decoding.
///
/// `index` follows the parser's attachment order. Text charset conversion is never applied;
/// malformed transfer encodings follow `mail-parser` recovery by returning their raw body.
/// For several reads from the same message, retain a [`ParsedMessage`] instead.
pub fn attachment_bytes(raw_message: &[u8], index: usize) -> Result<Vec<u8>, MimeError> {
    ParsedMessage::parse(raw_message)?.attachment_bytes(index)
}

/// Read one attachment from an existing parser view in normalized metadata order.
pub(crate) fn attachment_bytes_from_parsed(
    parsed: &CapturedMessage<'_>,
    index: usize,
) -> Result<Vec<u8>, MimeError> {
    let part = u32::try_from(index)
        .ok()
        .and_then(|attachment_index| parsed.attachment(attachment_index))
        .ok_or(MimeError::AttachmentNotFound { index })?;
    Ok(decoded_attachment_bytes(part, parsed.original_body(part)))
}

/// Decode one part from its original body range, preserving its pre-charset octets.
pub(crate) fn decoded_attachment_bytes(
    part: &mail_parser::MessagePart<'_>,
    encoded_body: &[u8],
) -> Vec<u8> {
    if part.is_encoding_problem {
        return encoded_body.to_vec();
    }

    let mut encoded_stream = MessageStream::new(encoded_body);
    match part.encoding {
        Encoding::Base64 => encoded_stream.decode_base64_mime(b"").1.into_owned(),
        Encoding::QuotedPrintable => encoded_stream
            .decode_quoted_printable_mime(b"")
            .1
            .into_owned(),
        Encoding::None => encoded_body.to_vec(),
    }
}

/// Describe every attachment using the exact bytes returned by [`attachment_bytes`].
pub(crate) fn attachment_metadata(parsed: &crate::parsing::CapturedMessage<'_>) -> Vec<Attachment> {
    parsed
        .attachments()
        .map(|part| {
            let attachment_bytes = decoded_attachment_bytes(part, parsed.original_body(part));
            let content_type = part
                .content_type()
                .map(|content_type| match content_type.subtype() {
                    Some(subtype) => format!("{}/{}", content_type.ctype(), subtype),
                    None => content_type.ctype().to_owned(),
                })
                .unwrap_or_else(|| {
                    if matches!(part.body, PartType::Message(_)) {
                        "message/rfc822".into()
                    } else {
                        "text/plain".into()
                    }
                });
            Attachment {
                filename: part.attachment_name().map(str::to_owned),
                content_type,
                size: attachment_bytes.len() as u64,
                content_hash: lowercase_hexadecimal(&Sha256::digest(&attachment_bytes)),
            }
        })
        .collect()
}

/// Encode bytes as lowercase hexadecimal, two digits per byte.
fn lowercase_hexadecimal(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}
