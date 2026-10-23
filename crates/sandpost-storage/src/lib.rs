mod capabilities;
mod error;
mod imap;
mod types;

pub use capabilities::{
    LinkCheckStorage, MessageStorage, OUTBOUND_ATTEMPT_RETENTION, OUTBOUND_DELIVERY_RETENTION,
    OutboundStorage, PERSONA_EXECUTION_RETENTION, PERSONA_PENDING_EXECUTION_LIMIT, PersonaStorage,
    ScopeStorage, SearchSynchronizationStorage, SmtpServerStorage, Storage, StorageHealth,
    UserStorage, ViewStorage,
};
pub use error::StorageError;
pub use imap::{
    ImapAccountCredential, ImapAccountSettings, ImapCopyResult, ImapFlagChange, ImapFlagChanges,
    ImapFlagOperation, ImapLocalFolderSettings, ImapMailboxCounts, ImapMailboxState, ImapMember,
    ImapMemberContent, ImapMemberMetadata, ImapMembershipDefinition, ImapStorage, NewImapAccount,
    NewImapLocalFolder, WebFolderSettings,
};
pub use types::{
    FilterExpression, IndexedMessage, MessageListQuery, MessageSummary, NewOutboundDelivery,
    NewOutboundRecipient, NewPersona, NewRoutingAction, NewSmtpAccess, NewSmtpOutbound, NewUser,
    OutboundAttemptResult, OutboundDeliveryQuery, OutboundDeliveryReader, OutboundRecipientUpdate,
    PersonaExecutionEntry, PersonaUpdate, RoutingActionUpdate, SearchOperation,
    SearchSynchronization, SmtpAccessCredential, SmtpOutboundCredential, SmtpOutboundPassword,
    SmtpOutboundSettings, UpdateUser,
};

pub use sandpost_core::{
    Attachment, GlobalRole, ImapAccount, ImapAccountIdentifier, ImapFolderIdentifier,
    ImapFolderKind, ImapLinkedView, ImapLocalFolder, ImapMailboxIdentifier, ImapMailboxSource,
    ImapMessageFlags, InboxIdentifier, LinkCheckErrorKind, LinkCheckResult, LinkOutcome,
    MailAccess, Mailbox, Message, MessageFacts, MessageIdentifier, MessageLinkCheck,
    MessageSequence, OutboundAttemptOutcome, OutboundDelivery, OutboundDeliveryAttempt,
    OutboundDeliveryIdentifier, OutboundDeliveryStatus, OutboundFailure, OutboundFailureKind,
    OutboundRecipient, OutboundRecipientStatus, Persona, PersonaActionDetail, PersonaActionKind,
    PersonaActionStatus, PersonaExecution, PersonaExecutionAction, PersonaExecutionIdentifier,
    PersonaExecutionStatus, PersonaHttpRequest, PersonaIdentifier, RoutingAction,
    RoutingActionIdentifier, RoutingActionKind, Scope, ScopeIdentifier, ScopeMembership, ScopeTree,
    SmtpAccess, SmtpAccessIdentifier, SmtpAuthenticationMechanism, SmtpOutbound,
    SmtpOutboundAuthentication, SmtpOutboundIdentifier, SmtpOutboundSecurity, User, UserIdentifier,
    View, ViewIdentifier, WebFolder,
};
