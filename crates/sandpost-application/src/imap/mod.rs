//! User-owned IMAP accounts: management, virtual mailboxes, and the session service the runtime
//! adapts to the IMAP protocol.
mod accounts;
mod catalog;
mod service;
mod session;
mod throttle;

pub use accounts::{
    ImapAccountDetails, ImapAccountUpdate, ImapFolderRequest, ImapMailboxPreview, ImapSecurity,
    ImapServerDetails, IssuedImapCredentials, NewImapAccountRequest,
};
pub use service::{ImapLogin, ImapService};
pub use session::{
    ImapAccessError, ImapAccountSession, ImapChangeWatch, ImapMailboxHandle, ImapMailboxListing,
    ImapMailboxSnapshot, ImapMailboxStatus, ImapMembershipChanges,
};

impl crate::Application {
    /// Record the connection details of the runtime's IMAP service for management interfaces.
    pub fn set_imap_server(&self, details: Option<ImapServerDetails>) {
        if let Ok(mut current) = self.imap_server.write() {
            *current = details;
        }
    }

    /// Create the IMAP authentication and session service over this application.
    pub fn imap_service(&self) -> ImapService {
        ImapService::new(self)
    }
}

#[cfg(test)]
mod tests;
