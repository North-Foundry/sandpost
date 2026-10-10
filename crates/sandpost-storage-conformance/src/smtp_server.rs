//! SMTP storage conformance checks grouped by listener and access behavior.
#[path = "smtp_server/access.rs"]
mod access;
#[path = "smtp_server/listener.rs"]
mod listener;

pub use access::{smtp_access_authentication_rules, smtp_access_credentials};
pub use listener::smtp_server_address;
