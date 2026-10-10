//! IMAP mailbox identity and membership definitions.
use crate::FilterExpression;
use sandpost_core::{ImapAccountIdentifier, ImapMailboxIdentifier, ImapMailboxSource};

/// The persistent identity and UID counters of one IMAP mailbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImapMailboxState {
    pub identifier: ImapMailboxIdentifier,
    pub account_identifier: ImapAccountIdentifier,
    pub source: ImapMailboxSource,
    /// Unique for this mailbox identity; never reused by a later mailbox.
    pub uid_validity: u32,
    /// The UID the next member will receive.
    pub uid_next: u32,
    /// The highest UID already reported as recent to a read-write session.
    pub recent_through: u32,
}

/// The complete membership definition of a mailbox at one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct ImapMembershipDefinition {
    /// Owner authorization AND the mailbox's own filter. A message is a member of a dynamic
    /// mailbox exactly when it matches; an explicit mailbox drops members that stop matching.
    pub filter: FilterExpression,
    /// Canonical fingerprint of `filter`; a change triggers a full reconciliation.
    pub fingerprint: String,
}
