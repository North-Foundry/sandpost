//! IMAP4rev1 syntax: commands, responses, and the data types they carry.
//!
//! These modules are public so tests and tools can exercise the grammar directly; servers use
//! them through [`ImapServer`](crate::ImapServer).
pub mod command;
pub mod datetime;
pub mod flags;
pub mod mailbox_name;
pub mod parser;
pub mod response;
pub mod sequence;
pub mod syntax;
