mod identifiers;
mod imap;
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
    ImapAccount, ImapFolderKind, ImapLinkedView, ImapLocalFolder, ImapMailboxSource,
    ImapMessageFlags, WebFolder,
};
pub use messages::{Attachment, Mailbox, Message, MessageFacts};
pub use scopes::{Scope, ScopeTree, TreeError};
pub use smtp_access::{SmtpAccess, SmtpAuthenticationMechanism};
pub use users::{GlobalRole, MailAccess, ScopeMembership, User, View};
