//! Reusable MIME interpretation for reading several parts from one captured message.

use crate::{
    MimeError,
    attachments::attachment_bytes_from_parsed,
    content_reference::content_reference_bytes_from_parsed,
    parsing::{CapturedMessage, parse_message},
};

/// A parsed MIME view that borrows the original message and can be reused for part reads.
///
/// Parse once when downloading several attachments or resolving several inline resources.
/// Reads transfer-decode the requested part without charset conversion or another MIME parse.
/// Recovery may allocate a temporary parser view; the original bytes remain unchanged.
/// This view interprets MIME parts. Use [`crate::RawMessage`] to produce normalized message facts
/// with envelope, receipt time and a configured size limit.
///
/// ```
/// use sandpost_mime::ParsedMessage;
///
/// let raw_message = b"Content-Type: application/octet-stream\r\n\
///                     Content-Disposition: attachment\r\n\
///                     Content-ID: <file@example.test>\r\n\r\ndata";
/// let parsed = ParsedMessage::parse(raw_message)?;
/// assert_eq!(parsed.attachment_count(), 1);
/// assert_eq!(parsed.attachment_bytes(0)?, b"data");
/// assert_eq!(parsed.content_reference_bytes("cid:file@example.test")?, b"data");
/// assert_eq!(parsed.raw_message(), raw_message);
/// # Ok::<(), sandpost_mime::MimeError>(())
/// ```
pub struct ParsedMessage<'a> {
    parsed: CapturedMessage<'a>,
}

impl std::fmt::Debug for ParsedMessage<'_> {
    /// Describe the parsed view without dumping captured message contents.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParsedMessage")
            .field("size", &self.raw_message().len())
            .field("attachment_count", &self.attachment_count())
            .finish_non_exhaustive()
    }
}

impl<'a> ParsedMessage<'a> {
    /// Interpret captured bytes once using the same receiver rules as [`crate::RawMessage`].
    pub fn parse(raw_message: &'a [u8]) -> Result<Self, MimeError> {
        Ok(Self {
            parsed: parse_message(raw_message)?,
        })
    }

    /// Return the original borrowed bytes, excluding any synthetic MIME recovery delimiters.
    #[must_use]
    pub fn raw_message(&self) -> &[u8] {
        self.parsed.original_message()
    }

    /// Return the number of attachments in the same order as normalized message metadata.
    #[must_use]
    pub fn attachment_count(&self) -> usize {
        self.parsed.attachments.len()
    }

    /// Read an attachment's transfer-decoded octets by zero-based metadata index.
    pub fn attachment_bytes(&self, index: usize) -> Result<Vec<u8>, MimeError> {
        attachment_bytes_from_parsed(&self.parsed, index)
    }

    /// Resolve a cid: or mid: URL inside this message and return transfer-decoded octets.
    ///
    /// Percent decoding preserves identifier case and plus signs. A matching mid: URL without
    /// a content identifier returns the whole original message. No external content is fetched.
    pub fn content_reference_bytes(&self, reference: &str) -> Result<Vec<u8>, MimeError> {
        content_reference_bytes_from_parsed(&self.parsed, reference)
    }
}
