//! Users storage types.
use sandpost_core::{GlobalRole, MailAccess};

/// A new user supplied to the user storage creation operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewUser {
    pub name: String,
    pub email: String,
    pub password_hash: String,
    pub global_role: GlobalRole,
    pub mail_access: MailAccess,
    pub personal_filter: Option<String>,
}

/// A complete replacement of a user's mutable fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateUser {
    pub name: String,
    pub email: String,
    pub password_hash: String,
    pub global_role: GlobalRole,
    pub mail_access: MailAccess,
    pub personal_filter: Option<String>,
}
