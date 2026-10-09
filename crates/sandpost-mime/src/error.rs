//! Why captured bytes could not become a message.
use thiserror::Error;

/// A message or mailbox that cannot be normalized.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MimeError {
    /// The message is larger than the configured size limit.
    #[error("message is {size} bytes, over the {limit}-byte limit")]
    MessageTooLarge { size: usize, limit: usize },
    /// The MIME parser found no header section at all.
    #[error("message has no parseable headers")]
    MissingHeaders,
    /// The parsed message has no attachment at the requested zero-based index.
    #[error("attachment {index} does not exist")]
    AttachmentNotFound { index: usize },
    /// A cid: or mid: reference has invalid syntax or percent encoding.
    #[error("invalid MIME content reference {0:?}")]
    InvalidContentReference(String),
    /// No part or message in the supplied MIME matches the requested reference.
    #[error("MIME content reference {0:?} does not exist")]
    ContentReferenceNotFound(String),
    /// An envelope or header address is not a usable `local@domain` mailbox.
    #[error("invalid mailbox {0:?}")]
    InvalidMailbox(String),
}
