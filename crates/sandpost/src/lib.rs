//! Embeddable Sandpost runtime shared by the server binary and native desktop shell.

mod bootstrap;
pub mod configuration;
mod imap_backend;
mod imap_listener;
mod ingestion;
mod lifecycle;
mod mail_listeners;
mod startup;

pub use configuration::{Configuration, ConfigurationError};
pub use lifecycle::{RunningSandpost, RuntimeError, SandpostRuntime};
pub use startup::StartupSummary;

#[cfg(test)]
mod imap_runtime_tests;
