//! User storage conformance checks grouped by lifecycle and role behavior.
#[path = "users/management.rs"]
mod management;
#[path = "users/roles.rs"]
mod roles;

pub use management::{duplicate_user_email, user_crud};
pub use roles::{bootstrap, global_roles, last_owner_invariant};
