//! IMAP flag changes and change-cursor results.
use sandpost_core::ImapMessageFlags;

/// How a flag change applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImapFlagOperation {
    Replace,
    Add,
    Remove,
}

/// A flag change applied to several members.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapFlagChange {
    pub operation: ImapFlagOperation,
    pub flags: ImapMessageFlags,
}

/// Members whose flags changed after a cursor, and the cursor that covers them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImapFlagChanges {
    pub changes: Vec<(u32, ImapMessageFlags)>,
    pub cursor: u64,
}
