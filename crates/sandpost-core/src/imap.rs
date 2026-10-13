//! User-owned IMAP accounts, their mailbox sources, IMAP-local folders, and message flags.
//!
//! An IMAP account belongs to one Sandpost user and never reads beyond that user's current mail
//! visibility. Its mailboxes project saved Views, owner-managed filters, or manual and private
//! folder placement; COPY and MOVE never duplicate the physical message.
use crate::{ImapAccountIdentifier, ImapFolderIdentifier, UserIdentifier, ViewIdentifier};
use serde::{Deserialize, Serialize};

/// One IMAP login of a Sandpost user, with its mailbox configuration.
///
/// The password exists only as a one-way hash in storage and is deliberately absent here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImapAccount {
    pub identifier: ImapAccountIdentifier,
    /// The Sandpost user whose mail visibility bounds this account.
    pub owner_identifier: UserIdentifier,
    /// Descriptive label chosen by the owner, such as "Laravel Debug".
    pub name: String,
    /// Unique IMAP login name.
    pub username: String,
    /// Disabled accounts neither authenticate nor keep running sessions.
    pub enabled: bool,
    /// Expose every saved View the owner can see; otherwise only the linked selection.
    pub mirror_views: bool,
    /// Unix seconds, UTC.
    pub created_at: i64,
    /// Unix seconds, UTC.
    pub updated_at: i64,
    /// Unix seconds, UTC, of the most recent successful login, if any.
    pub last_used_at: Option<i64>,
}

/// A saved View selected for an account that does not mirror every View.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImapLinkedView {
    pub view_identifier: ViewIdentifier,
    /// Mailbox name used instead of the View's own name; follows View renames when absent.
    pub alias: Option<String>,
}

/// One account projection of a dynamic folder managed for its Sandpost owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImapLocalFolder {
    pub identifier: ImapFolderIdentifier,
    /// The credential exposing this projection; the canonical definition belongs to its owner.
    pub account_identifier: ImapAccountIdentifier,
    pub name: String,
    /// Whether this account mailbox is dynamic, manually organized, or private to the account.
    pub kind: ImapFolderKind,
    /// Owner-managed definition identity; private folders have no definition.
    pub definition_identifier: Option<ImapFolderIdentifier>,
    /// Canonical query-language source evaluated over the owner's visible mail.
    pub filter: String,
    /// Whether the folder's messages also appear in the account's INBOX when the account does
    /// not mirror the owner's Views.
    pub include_in_inbox: bool,
    /// Unix seconds, UTC.
    pub created_at: i64,
    /// Unix seconds, UTC.
    pub updated_at: i64,
}

/// The organization model of one IMAP folder projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImapFolderKind {
    /// Membership follows an owner-managed filter.
    Smart,
    /// Membership follows Web placement, with account-local placement overrides.
    Manual,
    /// Membership and structure belong only to one IMAP account.
    Private,
}

/// A Web-managed manual folder owned by one Sandpost user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebFolder {
    pub identifier: ImapFolderIdentifier,
    pub owner_identifier: UserIdentifier,
    pub name: String,
    /// IMAP accounts that expose this folder; exposure does not grant mail access.
    pub exposed_accounts: Vec<ImapAccountIdentifier>,
    /// Unix seconds, UTC.
    pub created_at: i64,
    /// Unix seconds, UTC.
    pub updated_at: i64,
}

/// Where an IMAP mailbox takes its messages from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ImapMailboxSource {
    /// The aggregated INBOX.
    Inbox,
    /// The Trash, which holds messages explicitly moved or copied into it.
    Trash,
    /// A saved View of the owner.
    View(ViewIdentifier),
    /// An account projection of an owner-managed folder.
    LocalFolder(ImapFolderIdentifier),
}

impl ImapMailboxSource {
    /// Report whether messages enter the mailbox only by explicit copy rather than by filter.
    pub fn is_explicit(self) -> bool {
        matches!(self, Self::Trash)
    }
}

/// Persistent IMAP flags of one message in one mailbox.
///
/// `seen`, `answered`, `flagged`, `draft`, and the keywords are shared by every mailbox of the
/// account; `deleted` is reversible per membership until standalone expunge permanently
/// hides the message for the owning user across every interface.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImapMessageFlags {
    pub seen: bool,
    pub answered: bool,
    pub flagged: bool,
    pub draft: bool,
    pub deleted: bool,
    /// Client-defined keywords, deduplicated ASCII-case-insensitively.
    pub keywords: Vec<String>,
}
