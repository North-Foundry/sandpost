//! Turning captured bytes and their delivery context into a normalized message.
use crate::{
    Envelope, MimeError,
    attachments::attachment_metadata,
    headers::{collect_headers, header_mailboxes},
};
use mail_parser::{HeaderName, Message as ParsedMessage};
use sandpost_core::{Message, MessageFacts, MessageIdentifier};

/// The largest message [`RawMessage::parse`] accepts unless another limit is set: 10 MiB.
pub const DEFAULT_SIZE_LIMIT: usize = 10 * 1024 * 1024;

/// Captured message bytes together with how they arrived, ready to become a [`Message`].
///
/// The bytes are kept unchanged as the message's raw form. Parsing extracts decoded text and
/// HTML bodies, headers, the message ID, attachment metadata, and normalized mailboxes; it
/// accepts recoverable MIME input and does not promise strict MIME validation.
///
/// ```
/// use sandpost_mime::{Envelope, RawMessage, normalize_mailbox};
///
/// let message = RawMessage::new("From: app@example.test\r\nSubject: Hello\r\n\r\nHi.\r\n")
///     .envelope(Envelope::new().with_recipient(normalize_mailbox("qa@example.test")?))
///     .received_at(1_700_000_000)
///     .parse()?;
/// assert_eq!(message.facts.subject, "Hello");
/// assert_eq!(message.facts.envelope_to[0].address, "qa@example.test");
/// # Ok::<(), sandpost_mime::MimeError>(())
/// ```
#[derive(Debug, Clone)]
#[must_use = "a raw message does nothing until it is parsed"]
pub struct RawMessage {
    bytes: Vec<u8>,
    envelope: Envelope,
    received_at: Option<i64>,
    size_limit: usize,
}

impl RawMessage {
    /// Wrap captured bytes with an empty envelope, the current time, and the default size limit.
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: bytes.into(),
            envelope: Envelope::new(),
            received_at: None,
            size_limit: DEFAULT_SIZE_LIMIT,
        }
    }

    /// Attach the SMTP envelope the message arrived with.
    pub fn envelope(mut self, envelope: Envelope) -> Self {
        self.envelope = envelope;
        self
    }

    /// Record when the message arrived, in Unix seconds; defaults to the time of parsing.
    pub fn received_at(mut self, unix_seconds: i64) -> Self {
        self.received_at = Some(unix_seconds);
        self
    }

    /// Refuse messages larger than `bytes`; defaults to [`DEFAULT_SIZE_LIMIT`].
    pub fn size_limit(mut self, bytes: usize) -> Self {
        self.size_limit = bytes;
        self
    }

    /// Parse the bytes into a message with a new identifier.
    ///
    /// Fails when the message exceeds the size limit, cannot be recovered by the parser,
    /// or yields an address header mailbox with invalid addr-spec syntax.
    pub fn parse(self) -> Result<Message, MimeError> {
        let Self {
            bytes,
            envelope,
            received_at,
            size_limit,
        } = self;
        if bytes.len() > size_limit {
            return Err(MimeError::MessageTooLarge {
                size: bytes.len(),
                limit: size_limit,
            });
        }
        let parsed = crate::parsing::parse_message(&bytes)?;
        let attachments = attachment_metadata(&parsed);
        let facts = MessageFacts {
            envelope_from: envelope.sender,
            envelope_to: envelope.recipients,
            from: header_mailboxes(&parsed, HeaderName::From)?,
            to: header_mailboxes(&parsed, HeaderName::To)?,
            carbon_copy: header_mailboxes(&parsed, HeaderName::Cc)?,
            subject: parsed.subject().unwrap_or_default().to_owned(),
            text: text_bodies(&parsed),
            markup_body: markup_bodies(&parsed),
            message_identifier: parsed.message_id().map(str::to_owned),
            received_at: received_at.unwrap_or_else(current_unix_seconds),
            size: bytes.len() as u64,
            attachment_count: attachments.len() as u64,
            headers: collect_headers(&parsed, parsed.raw_message.as_ref()),
        };
        drop(parsed);
        Ok(Message {
            identifier: MessageIdentifier::new(),
            facts,
            raw_message: bytes,
            attachments,
        })
    }
}

/// Join every decoded plain-text body part with newlines.
fn text_bodies(parsed: &ParsedMessage<'_>) -> String {
    (0..parsed.text_body_count())
        .filter_map(|body_part_index| parsed.body_text(body_part_index))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Join every decoded HTML body part with newlines.
fn markup_bodies(parsed: &ParsedMessage<'_>) -> String {
    (0..parsed.html_body_count())
        .filter_map(|body_part_index| parsed.body_html(body_part_index))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Return the current Unix timestamp in whole seconds.
fn current_unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
