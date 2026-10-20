mod capabilities;
mod error;
mod imap;
mod types;

pub use capabilities::{
    LinkCheckStorage, MessageStorage, PERSONA_EXECUTION_RETENTION, PERSONA_PENDING_EXECUTION_LIMIT,
    PersonaStorage, ScopeStorage, SearchSynchronizationStorage, SmtpServerStorage, Storage,
    StorageHealth, UserStorage, ViewStorage,
};
pub use error::StorageError;
pub use imap::{
    ImapAccountCredential, ImapAccountSettings, ImapCopyResult, ImapFlagChange, ImapFlagChanges,
    ImapFlagOperation, ImapLocalFolderSettings, ImapMailboxCounts, ImapMailboxState, ImapMember,
    ImapMemberContent, ImapMemberMetadata, ImapMembershipDefinition, ImapStorage, NewImapAccount,
    NewImapLocalFolder, WebFolderSettings,
};
pub use types::{
    FilterExpression, IndexedMessage, MessageListQuery, MessageSummary, NewPersona, NewSmtpAccess,
    NewUser, PersonaExecutionEntry, PersonaUpdate, SearchOperation, SearchSynchronization,
    SmtpAccessCredential, UpdateUser,
};

pub use sandpost_core::{
    Attachment, GlobalRole, ImapAccount, ImapAccountIdentifier, ImapFolderIdentifier,
    ImapFolderKind, ImapLinkedView, ImapLocalFolder, ImapMailboxIdentifier, ImapMailboxSource,
    ImapMessageFlags, InboxIdentifier, LinkCheckErrorKind, LinkCheckResult, LinkOutcome,
    MailAccess, Mailbox, Message, MessageFacts, MessageIdentifier, MessageLinkCheck,
    MessageSequence, Persona, PersonaActionDetail, PersonaActionKind, PersonaActionStatus,
    PersonaExecution, PersonaExecutionAction, PersonaExecutionIdentifier, PersonaExecutionStatus,
    PersonaHttpRequest, PersonaIdentifier, Scope, ScopeIdentifier, ScopeMembership, ScopeTree,
    SmtpAccess, SmtpAccessIdentifier, SmtpAuthenticationMechanism, User, UserIdentifier, View,
    ViewIdentifier, WebFolder,
};
