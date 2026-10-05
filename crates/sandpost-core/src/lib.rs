#![doc = include_str!("../README.md")]

mod identifiers;
mod messages;
mod scopes;
mod users;

pub use identifiers::{
    EndpointIdentifier, InboxIdentifier, MessageIdentifier, MessageSequence, ScopeIdentifier,
    UserIdentifier, ViewIdentifier,
};
pub use messages::{Attachment, Mailbox, Message, MessageFacts};
pub use scopes::{Scope, ScopeTree, TreeError, default_endpoint_identifier};
pub use users::{
    EndpointMembership, EndpointRole, GlobalRole, Inbox, MailAccess, ScopeMembership, SmtpEndpoint,
    User, View,
};
