//! The SMTP envelope that travels with a message.
use sandpost_core::Mailbox;

/// Who submitted a message and whom it was delivered to, as stated by the SMTP transaction.
///
/// The envelope is independent of the `From`, `To`, and `Cc` headers, which the sender writes
/// freely; both are kept on the parsed message.
///
/// ```
/// use sandpost_mime::{Envelope, normalize_mailbox};
///
/// let envelope = Envelope::new()
///     .with_sender(normalize_mailbox("app@example.test")?)
///     .with_recipient(normalize_mailbox("qa@example.test")?);
/// assert_eq!(envelope.recipients.len(), 1);
/// # Ok::<(), sandpost_mime::MimeError>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Envelope {
    /// The reverse path; `None` is the null sender `<>` used by bounces.
    pub sender: Option<Mailbox>,
    /// The forward paths, in the order they were accepted.
    pub recipients: Vec<Mailbox>,
}

impl Envelope {
    /// Start an envelope with the null sender and no recipients.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the reverse path.
    #[must_use]
    pub fn with_sender(mut self, sender: Mailbox) -> Self {
        self.sender = Some(sender);
        self
    }

    /// Append one forward path.
    #[must_use]
    pub fn with_recipient(mut self, recipient: Mailbox) -> Self {
        self.recipients.push(recipient);
        self
    }

    /// Append several forward paths in order.
    #[must_use]
    pub fn with_recipients(mut self, recipients: impl IntoIterator<Item = Mailbox>) -> Self {
        self.recipients.extend(recipients);
        self
    }
}
