mod identifiers;
mod imap;
mod inbox_addresses;
mod link_checks;
mod messages;
mod outbound;
mod personas;
mod scopes;
mod smtp_access;
mod users;

pub use identifiers::{
    ImapAccountIdentifier, ImapFolderIdentifier, ImapMailboxIdentifier, InboxIdentifier,
    MessageIdentifier, MessageSequence, OutboundDeliveryIdentifier, PersonaExecutionIdentifier,
    PersonaIdentifier, RoutingActionIdentifier, ScopeIdentifier, SmtpAccessIdentifier,
    SmtpOutboundIdentifier, UserIdentifier, ViewIdentifier,
};
pub use imap::{
    ImapAccount, ImapAccountMode, ImapFolderKind, ImapLinkedView, ImapLocalFolder,
    ImapMailboxSource, ImapMessageFlags, WebFolder,
};
pub use inbox_addresses::normalize_inbox_address;
pub use link_checks::{LinkCheckErrorKind, LinkCheckResult, LinkOutcome, MessageLinkCheck};
pub use messages::{Attachment, Mailbox, Message, MessageFacts};
pub use outbound::{
    OutboundAttemptOutcome, OutboundDelivery, OutboundDeliveryAttempt, OutboundDeliveryStatus,
    OutboundFailure, OutboundFailureKind, OutboundRecipient, OutboundRecipientStatus,
    RoutingAction, RoutingActionKind, SmtpOutbound, SmtpOutboundAuthentication,
    SmtpOutboundSecurity,
};
pub use personas::{
    Persona, PersonaActionDetail, PersonaActionKind, PersonaActionStatus, PersonaExecution,
    PersonaExecutionAction, PersonaExecutionStatus, PersonaHttpRequest,
};
pub use scopes::{Scope, ScopeTree, TreeError};
pub use smtp_access::{SmtpAccess, SmtpAuthenticationMechanism};
pub use users::{GlobalRole, MailAccess, ScopeMembership, User, View};
