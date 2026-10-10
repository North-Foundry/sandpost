#![doc = include_str!("../README.md")]

mod capabilities;
mod error;
mod imap;
mod types;

pub use capabilities::{
    MessageStorage, ScopeStorage, SearchSynchronizationStorage, SmtpServerStorage, Storage,
    StorageHealth, UserStorage, ViewStorage,
};
pub use error::StorageError;
pub use imap::{
    ImapAccountCredential, ImapAccountSettings, ImapCopyResult, ImapFlagChange, ImapFlagChanges,
    ImapFlagOperation, ImapLocalFolderSettings, ImapMailboxCounts, ImapMailboxState, ImapMember,
    ImapMemberContent, ImapMemberMetadata, ImapMembershipDefinition, ImapStorage, NewImapAccount,
    NewImapLocalFolder,
};
pub use types::{
    FilterExpression, IndexedMessage, MessageListQuery, MessageSummary, NewSmtpAccess, NewUser,
    SearchOperation, SearchSynchronization, SmtpAccessCredential, UpdateUser,
};

pub use sandpost_core::{
    Attachment, GlobalRole, ImapAccount, ImapAccountIdentifier, ImapFolderIdentifier,
    ImapLinkedView, ImapLocalFolder, ImapMailboxIdentifier, ImapMailboxSource, ImapMessageFlags,
    Inbox, InboxIdentifier, MailAccess, Mailbox, Message, MessageFacts, MessageIdentifier,
    MessageSequence, Scope, ScopeIdentifier, ScopeMembership, ScopeTree, SmtpAccess,
    SmtpAccessIdentifier, SmtpAuthenticationMechanism, User, UserIdentifier, View, ViewIdentifier,
};
