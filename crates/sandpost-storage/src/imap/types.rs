//! Backend-independent IMAP storage request and result types.

mod account;
mod copy;
mod flags;
mod folders;
mod mailbox;
mod members;

pub use account::{ImapAccountCredential, ImapAccountSettings, NewImapAccount};
pub use copy::ImapCopyResult;
pub use flags::{ImapFlagChange, ImapFlagChanges, ImapFlagOperation};
pub use folders::{ImapLocalFolderSettings, NewImapLocalFolder};
pub use mailbox::{ImapMailboxState, ImapMembershipDefinition};
pub use members::{ImapMailboxCounts, ImapMember, ImapMemberContent, ImapMemberMetadata};
