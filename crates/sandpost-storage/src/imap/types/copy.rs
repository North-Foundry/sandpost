//! IMAP copy and move results.

/// The result of copying or moving members between mailboxes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImapCopyResult {
    pub uid_validity: u32,
    /// Copied source UIDs, positionally matching `destination_uids`.
    pub source_uids: Vec<u32>,
    /// Destination UIDs: newly assigned, or the existing UID of a message already present.
    pub destination_uids: Vec<u32>,
    /// Whether every destination UID was newly assigned by this copy.
    pub all_new: bool,
    /// Source UIDs removed by a move.
    pub removed_uids: Vec<u32>,
}
