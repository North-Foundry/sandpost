//! Real-runtime IMAP checks grouped by message capture, access, organization, and lifetime.

use crate::{RunningSandpost, SandpostRuntime};
use sandpost_application::{
    ImapFolderRequest, IssuedImapCredentials, LoginRequest, NewImapAccountRequest, SessionUser,
};
use sandpost_core::{GlobalRole, MailAccess, Scope, ScopeIdentifier};
use support::*;
use tokio::io::AsyncWriteExt;

mod authorization;
mod capture;
mod mailboxes;
mod modes;
mod retention;
mod smart_mailboxes;
mod subscriptions;
mod support;
