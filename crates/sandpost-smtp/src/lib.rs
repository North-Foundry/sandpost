#![doc = include_str!("../README.md")]

mod framing;
mod handler;
mod limits;
mod server;
mod session;
mod tls;

pub use handler::{
    AuthenticationError, AuthenticationOutcome, AuthenticationPolicy, Credentials, DeliveryError,
    SessionHandler,
};
pub use limits::Limits;
pub use sandpost_core::SmtpAuthenticationMechanism;
pub use server::SmtpServer;
pub use tls::{TlsConfiguration, TlsConfigurationError, TransportSecurity};

#[cfg(test)]
mod tests;
