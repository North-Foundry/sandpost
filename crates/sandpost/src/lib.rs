//! Embeddable Sandpost runtime shared by the server binary and native desktop shell.

mod bootstrap;
pub mod configuration;
mod frontend_proxy;
mod imap_backend;
mod imap_listener;
mod lifecycle;
mod maintenance;
mod network;
mod smtp_delivery;
mod smtp_listener;
mod startup;

pub use configuration::{Configuration, ConfigurationError, PersonaConfiguration};
pub use lifecycle::{RunningSandpost, RuntimeError, SandpostRuntime};
pub use maintenance::{MaintenanceCommand, MigrationOperation};
pub use startup::StartupSummary;

#[cfg(test)]
mod imap_runtime_tests;
