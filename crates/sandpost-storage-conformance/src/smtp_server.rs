//! SMTP access storage conformance checks.
#[path = "smtp_server/access.rs"]
mod access;

pub use access::{smtp_access_authentication_rules, smtp_access_credentials};
