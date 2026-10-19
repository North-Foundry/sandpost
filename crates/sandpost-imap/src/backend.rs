//! The contract between the IMAP protocol and the application that owns accounts and mail.
//!
//! The protocol layer frames commands, tracks session state (message sequence numbers, the
//! selected mailbox, `\Recent`), and renders message data. Everything about who may log in,
//! which mailboxes exist, which messages they contain, their UIDs, and their persistent flags
//! belongs to the application, which implements [`ImapBackend`] and [`Account`].
//!
//! Backends must keep UIDs stable and strictly ascending within a mailbox for a given
//! UIDVALIDITY, never reuse a UID, and enforce authorization on every call: the protocol layer
//! never decides whether a message may be shown.
use crate::protocol::flags::{FlagChange, Flags};
use std::{future::Future, net::IpAddr, sync::Arc};
use thiserror::Error;

/// A message's unique identifier within one mailbox and UIDVALIDITY (RFC 3501 section 2.3.1.1).
pub type Uid = u32;

/// How credentials were presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationMechanism {
    /// The LOGIN command.
    Login,
    /// `AUTHENTICATE PLAIN` (RFC 4616).
    Plain,
}

/// Credentials presented by a client, with the circumstances of the attempt.
///
/// Intentionally lacks Debug and Display so passwords are never formatted into logs.
pub struct Credentials {
    pub username: String,
    pub password: String,
    pub mechanism: AuthenticationMechanism,
    /// Whether the connection was protected by TLS when the credentials arrived.
    pub encrypted: bool,
    /// The client's address, for rate limiting; absent for in-memory connections.
    pub remote_address: Option<IpAddr>,
}

/// The backend's verdict on presented credentials.
pub enum AuthenticationOutcome<A> {
    /// Valid credentials; the session is bound to this account.
    Authenticated(A),
    /// Unknown, wrong, or disabled credentials (`NO [AUTHENTICATIONFAILED]`).
    InvalidCredentials,
    /// Too many recent failures; refused without verifying (`NO [UNAVAILABLE]`).
    Throttled,
}

/// A failure that is not about a specific mailbox.
#[derive(Debug, Error)]
pub enum BackendError {
    /// A temporary failure such as unavailable storage (`NO [UNAVAILABLE]`).
    #[error("temporarily unavailable: {0}")]
    Unavailable(String),
    /// The account was disabled, deleted, or had its credentials replaced; the session ends
    /// with `BYE`.
    #[error("the account is no longer available")]
    AccountRevoked,
}

/// Why a mailbox operation failed.
#[derive(Debug, Error)]
pub enum MailboxError {
    /// No mailbox with that name is visible to the account (`NO [NONEXISTENT]`, or
    /// `NO [TRYCREATE]` as a COPY or MOVE destination).
    #[error("mailbox does not exist")]
    NonExistent,
    /// The operation is not possible for this mailbox (`NO [CANNOT]`), with an explanation.
    #[error("{0}")]
    NotPermitted(String),
    /// A failure not specific to the mailbox.
    #[error(transparent)]
    Backend(#[from] BackendError),
}

/// A special use of a mailbox announced in LIST responses (RFC 6154 section 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialUse {
    All,
    Archive,
    Drafts,
    Flagged,
    Junk,
    Sent,
    Trash,
}

impl SpecialUse {
    /// The mailbox attribute, including its leading backslash.
    pub fn attribute(self) -> &'static str {
        match self {
            Self::All => "\\All",
            Self::Archive => "\\Archive",
            Self::Drafts => "\\Drafts",
            Self::Flagged => "\\Flagged",
            Self::Junk => "\\Junk",
            Self::Sent => "\\Sent",
            Self::Trash => "\\Trash",
        }
    }
}

/// One mailbox visible to an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxListing {
    /// The Unicode name; the protocol layer encodes it in modified UTF-7.
    pub name: String,
    /// Whether the name currently identifies a selectable mailbox.
    pub selectable: bool,
    pub special_use: Option<SpecialUse>,
    pub subscribed: bool,
}

/// One message as the session sees it: UID, persistent flags, and whether it is recent for
/// this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageState {
    pub uid: Uid,
    pub flags: Flags,
    pub recent: bool,
}

/// The state of a mailbox when it is selected.
#[derive(Debug, Clone)]
pub struct MailboxSnapshot<Handle> {
    /// An opaque handle the backend uses for later operations on this mailbox.
    pub handle: Handle,
    pub uid_validity: u32,
    pub uid_next: Uid,
    /// Every message in ascending UID order.
    pub messages: Vec<MessageState>,
    /// Whether this mailbox permits only reads and changes to `\\Seen`.
    pub seen_only: bool,
    /**
     * Whether the account role makes this selection read-only, even for SELECT.
     */
    pub read_only: bool,
    /// A cursor identifying the flag state included in this snapshot.
    pub cursor: u64,
}

/// What a session already knows, so the backend can report only changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SynchronizationRequest {
    /// The highest UID the session knows, or zero.
    pub highest_uid: Uid,
    /// How many of the messages with UIDs up to `highest_uid` the session believes still exist.
    pub known_count: u32,
    /// The cursor of the latest flag state the session has seen.
    pub cursor: u64,
    /// Whether newly arrived messages may become recent for this session (read-write only).
    pub claim_recent: bool,
}

/// Changes since a [`SynchronizationRequest`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MailboxChanges {
    /// When some known message is gone, every UID up to the requested `highest_uid` that still
    /// exists, in ascending order; `None` when nothing known was removed.
    pub present_uids: Option<Vec<Uid>>,
    /// Messages with UIDs above the requested `highest_uid`, in ascending UID order.
    pub added: Vec<MessageState>,
    /// Current flags of known messages whose flags may have changed since the cursor.
    pub flag_updates: Vec<(Uid, Flags)>,
    pub uid_next: Uid,
    pub cursor: u64,
    /**
     * Whether the account role currently makes this selected mailbox read-only.
     */
    pub read_only: bool,
}

/// STATUS data of a mailbox (RFC 3501 section 6.3.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailboxStatus {
    pub messages: u32,
    pub recent: u32,
    pub uid_next: Uid,
    pub uid_validity: u32,
    pub unseen: u32,
}

/// Size and internal date of a message, which FETCH and SEARCH may need without its octets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageMetadata {
    pub uid: Uid,
    /// RFC822.SIZE: the exact number of octets of the message.
    pub size: u64,
    /// INTERNALDATE as Unix seconds.
    pub internal_date: i64,
}

/// The octets and internal date of one message.
#[derive(Debug, Clone)]
pub struct MessageContent {
    pub octets: Arc<[u8]>,
    pub internal_date: i64,
}

/// The result of COPY or MOVE (RFC 4315 COPYUID, RFC 6851).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CopyOutcome {
    pub uid_validity: u32,
    /// Copied source UIDs; positionally matching `destination_uids` when that is not empty.
    pub source_uids: Vec<Uid>,
    /// Newly assigned destination UIDs, or empty when the backend assigned none.
    pub destination_uids: Vec<Uid>,
    /// Source UIDs removed from the selected mailbox (MOVE only).
    pub expunged: Vec<Uid>,
}

/// A subscription to changes that may affect what an account sees.
///
/// Implementations must be cancel-safe: the session waits on [`ChangeWatch::changed`] inside
/// `select!` and drops the future whenever input arrives first.
pub trait ChangeWatch: Send + 'static {
    /// Wait for a possible change. Returns `false` when no further notifications will come.
    fn changed(&mut self) -> impl Future<Output = bool> + Send;
}

/// Authenticates IMAP sessions.
pub trait ImapBackend: Send + Sync + 'static {
    /// The account bound to an authenticated session.
    type Account: Account;

    /// Verify credentials. Returns an error only for temporary failures.
    fn authenticate(
        &self,
        credentials: Credentials,
    ) -> impl Future<Output = Result<AuthenticationOutcome<Self::Account>, BackendError>> + Send;
}

/// One authenticated account: its mailboxes, their messages, and persistent flags.
pub trait Account: Send + Sync + 'static {
    /// Opaque handle of a selected mailbox.
    type Mailbox: Clone + Send + Sync + 'static;
    /// Subscription to changes, used while the client idles.
    type Watch: ChangeWatch;

    /// Confirm the account may continue; [`BackendError::AccountRevoked`] ends the session.
    fn validate(&self) -> impl Future<Output = Result<(), BackendError>> + Send;

    /// List every mailbox visible to the account.
    fn mailboxes(&self) -> impl Future<Output = Result<Vec<MailboxListing>, BackendError>> + Send;

    /// Subscribe to or unsubscribe from a mailbox name.
    fn set_subscription(
        &self,
        name: &str,
        subscribed: bool,
    ) -> impl Future<Output = Result<(), MailboxError>> + Send;

    /// Select a mailbox. A read-write selection claims newly recent messages for the session.
    fn open(
        &self,
        name: &str,
        read_only: bool,
    ) -> impl Future<Output = Result<MailboxSnapshot<Self::Mailbox>, MailboxError>> + Send;

    /// Report STATUS data of a mailbox without selecting it.
    fn status(
        &self,
        name: &str,
    ) -> impl Future<Output = Result<MailboxStatus, MailboxError>> + Send;

    /// Report changes to a selected mailbox since what the session already knows.
    ///
    /// [`MailboxError::NonExistent`] means the mailbox disappeared or became inaccessible; the
    /// session then removes every message it knows.
    fn synchronize(
        &self,
        mailbox: &Self::Mailbox,
        request: SynchronizationRequest,
    ) -> impl Future<Output = Result<MailboxChanges, MailboxError>> + Send;

    /// Load size and internal date of messages; UIDs no longer in the mailbox are omitted.
    fn metadata(
        &self,
        mailbox: &Self::Mailbox,
        uids: &[Uid],
    ) -> impl Future<Output = Result<Vec<MessageMetadata>, MailboxError>> + Send;

    /// Load one message's octets, or `None` when it is no longer in the mailbox or no longer
    /// visible to the account.
    fn content(
        &self,
        mailbox: &Self::Mailbox,
        uid: Uid,
    ) -> impl Future<Output = Result<Option<MessageContent>, MailboxError>> + Send;

    /// Change persistent flags and return the resulting flags of every changed or unchanged
    /// message that still exists.
    fn store(
        &self,
        mailbox: &Self::Mailbox,
        uids: &[Uid],
        change: &FlagChange,
    ) -> impl Future<Output = Result<Vec<(Uid, Flags)>, MailboxError>> + Send;

    /// Remove messages flagged `\Deleted` from the mailbox, restricted to `uids` when given, and
    /// return the removed UIDs.
    fn expunge(
        &self,
        mailbox: &Self::Mailbox,
        uids: Option<&[Uid]>,
    ) -> impl Future<Output = Result<Vec<Uid>, MailboxError>> + Send;

    /// Copy messages to another mailbox, removing them from the source for MOVE.
    fn copy(
        &self,
        mailbox: &Self::Mailbox,
        uids: &[Uid],
        destination: &str,
        remove_from_source: bool,
    ) -> impl Future<Output = Result<CopyOutcome, MailboxError>> + Send;

    /// Create a mailbox; refused unless the backend supports it.
    fn create(&self, _name: &str) -> impl Future<Output = Result<(), MailboxError>> + Send {
        std::future::ready(Err(MailboxError::NotPermitted(
            "mailboxes cannot be created through IMAP".into(),
        )))
    }

    /// Delete a mailbox; refused unless the backend supports it.
    fn delete(&self, _name: &str) -> impl Future<Output = Result<(), MailboxError>> + Send {
        std::future::ready(Err(MailboxError::NotPermitted(
            "mailboxes cannot be deleted through IMAP".into(),
        )))
    }

    /// Rename a mailbox; refused unless the backend supports it.
    fn rename(
        &self,
        _source: &str,
        _destination: &str,
    ) -> impl Future<Output = Result<(), MailboxError>> + Send {
        std::future::ready(Err(MailboxError::NotPermitted(
            "mailboxes cannot be renamed through IMAP".into(),
        )))
    }

    /// Subscribe to changes that may affect this account.
    fn watch(&self) -> Self::Watch;
}
