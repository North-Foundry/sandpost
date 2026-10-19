mod identifiers;
mod imap;
mod inbox_addresses;
mod link_checks;
mod messages;
mod scopes;
mod smtp_access;
mod users;

pub use identifiers::{
    ImapAccountIdentifier, ImapFolderIdentifier, ImapMailboxIdentifier, InboxIdentifier,
    MessageIdentifier, MessageSequence, ScopeIdentifier, SmtpAccessIdentifier, UserIdentifier,
    ViewIdentifier,
};
pub use imap::{
    ImapAccount, ImapAccountMode, ImapFolderKind, ImapLinkedView, ImapLocalFolder,
    ImapMailboxSource, ImapMessageFlags, WebFolder,
};
pub use inbox_addresses::normalize_inbox_address;
pub use link_checks::{LinkCheckErrorKind, LinkCheckResult, LinkOutcome, MessageLinkCheck};
pub use messages::{Attachment, Mailbox, Message, MessageFacts};
pub use scopes::{Scope, ScopeTree, TreeError};
pub use smtp_access::{SmtpAccess, SmtpAuthenticationMechanism};
pub use users::{GlobalRole, MailAccess, ScopeMembership, User, View};
