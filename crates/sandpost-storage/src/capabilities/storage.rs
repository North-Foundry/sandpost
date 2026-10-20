//! Composite storage dependency.
use crate::{
    ImapStorage, LinkCheckStorage, MessageStorage, PersonaStorage, ScopeStorage,
    SearchSynchronizationStorage, SmtpServerStorage, StorageHealth, UserStorage, ViewStorage,
};

/// The complete storage dependency used by application and transport layers.
///
/// A concrete backend implements the capability traits above; consumers depend only on Storage
/// (typically through an Arc). Every capability is object-safe, so one trait object serves the
/// whole application.
pub trait Storage:
    UserStorage
    + SmtpServerStorage
    + ImapStorage
    + ScopeStorage
    + ViewStorage
    + MessageStorage
    + LinkCheckStorage
    + PersonaStorage
    + SearchSynchronizationStorage
    + StorageHealth
{
}

impl<T> Storage for T where
    T: UserStorage
        + SmtpServerStorage
        + ImapStorage
        + ScopeStorage
        + ViewStorage
        + MessageStorage
        + LinkCheckStorage
        + PersonaStorage
        + SearchSynchronizationStorage
        + StorageHealth
{
}
