//! IMAP copy and move results.

/// The result of copying or moving members between mailboxes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImapCopyResult {
    pub uid_validity: u32,
    /// Copied source UIDs, positionally matching `destination_uids`.
    pub source_uids: Vec<u32>,
    /// Fresh destination UIDs, positionally matching the copied source UIDs.
    pub destination_uids: Vec<u32>,
    /// Whether every destination UID was newly assigned by this copy.
    pub all_new: bool,
    /// Source UIDs removed by a move.
    pub removed_uids: Vec<u32>,
}
