//! IMAP mailbox member projections and counts.
use sandpost_core::{ImapMessageFlags, MessageIdentifier};

/// One member of a mailbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapMember {
    pub uid: u32,
    pub message_identifier: MessageIdentifier,
    pub flags: ImapMessageFlags,
}

/// The size and internal date of a member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImapMemberMetadata {
    pub uid: u32,
    /// Exact octets of the stored message.
    pub size: u64,
    /// Unix seconds, UTC, when the message was received.
    pub received_at: i64,
}

/// The stored octets of a member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapMemberContent {
    pub uid: u32,
    pub message_identifier: MessageIdentifier,
    pub raw_message: Vec<u8>,
    /// Unix seconds, UTC.
    pub received_at: i64,
}

/// Counts used by STATUS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImapMailboxCounts {
    pub messages: u32,
    pub unseen: u32,
    /// Members with a UID above the mailbox's recent watermark.
    pub recent: u32,
}
