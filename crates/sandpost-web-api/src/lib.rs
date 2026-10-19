//! Versioned HTTP transport for the local development catcher.

mod authentication;
mod embedded;
mod error;
mod events;
mod imap;
mod inboxes;
mod link_checks;
mod mailbox;
mod management;
mod messages;
mod read_status;
mod routing;
mod smtp;

pub use embedded::EMBEDDED_FRONTEND;
pub use routing::{router, router_with_embedded_frontend, router_with_frontend};
pub use sandpost_application::{Application as ApplicationState, MessageEvent};

#[cfg(test)]
mod tests;
