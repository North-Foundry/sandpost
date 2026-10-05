#![doc = include_str!("../README.md")]

mod capabilities;
mod error;
mod types;

pub use capabilities::{
    EndpointStorage, MessageStorage, ScopeStorage, SearchSynchronizationStorage, Storage,
    StorageHealth, UserStorage, ViewStorage,
};
pub use error::StorageError;
pub use types::{
    FilterExpression, IndexedMessage, MessageListQuery, MessageSummary, NewUser, SearchOperation,
    SearchSynchronization, UpdateUser,
};

pub use sandpost_core::{
    Attachment, EndpointIdentifier, EndpointMembership, EndpointRole, GlobalRole, Inbox,
    InboxIdentifier, MailAccess, Mailbox, Message, MessageFacts, MessageIdentifier,
    MessageSequence, Scope, ScopeIdentifier, ScopeMembership, ScopeTree, SmtpEndpoint, User,
    UserIdentifier, View, ViewIdentifier, default_endpoint_identifier,
};
