//! IMAP storage conformance checks grouped by account, mailbox, and flag behavior.
#[path = "imap/accounts.rs"]
mod accounts;
#[path = "imap/flags.rs"]
mod flags;
#[path = "imap/mailboxes.rs"]
mod mailboxes;
#[path = "imap/manual.rs"]
mod manual;
#[path = "imap/support.rs"]
mod support;

pub use accounts::{imap_account_configuration, imap_account_credentials};
pub use flags::imap_flags_and_expunge;
pub use mailboxes::{imap_mailbox_identities, imap_membership_synchronization};
pub use manual::{imap_copy_then_expunge_transfers, web_manual_folders_and_placements, web_trash};
