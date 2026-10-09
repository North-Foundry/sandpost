#![doc = include_str!("../README.md")]

mod attachments;
mod boundaries;
mod content_reference;
mod envelope;
mod error;
mod headers;
mod mailbox;
mod parsed_message;
mod parsing;
mod raw_message;

pub use attachments::attachment_bytes;
pub use content_reference::content_reference_bytes;
pub use envelope::Envelope;
pub use error::MimeError;
pub use mailbox::normalize_mailbox;
pub use parsed_message::ParsedMessage;
pub use raw_message::{DEFAULT_SIZE_LIMIT, RawMessage};
