//! Versioned HTTP transport for the local development catcher.

mod authentication;
mod error;
mod events;
mod imap;
mod inboxes;
mod management;
mod messages;
mod routing;
mod smtp;

pub use routing::{router, router_with_frontend};
pub use sandpost_application::{Application as ApplicationState, MessageEvent};

#[cfg(test)]
mod tests;
