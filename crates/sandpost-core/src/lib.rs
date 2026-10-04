#![doc = include_str!("../README.md")]

mod identifiers;
mod messages;
mod scopes;
mod users;

pub use identifiers::{
    InboxIdentifier, MessageIdentifier, MessageSequence, ScopeIdentifier, UserIdentifier,
};
pub use messages::{Attachment, Mailbox, Message, MessageFacts};
pub use scopes::{Scope, ScopeTree, TreeError};
pub use users::{Inbox, Membership, Role, User};
