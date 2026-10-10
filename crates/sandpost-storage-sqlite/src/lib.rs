#![doc = include_str!("../README.md")]

mod connection;
mod error;
mod filter;
mod imap;
mod messages;
mod migrations;
mod records;
mod schema;
mod scopes;
mod search_sync;
mod smtp_server;
mod users;
mod views;

pub use connection::SqliteStorage;
pub use sandpost_storage::{
    ImapStorage, MessageStorage, ScopeStorage, SearchSynchronizationStorage, SmtpServerStorage,
    Storage, StorageError, StorageHealth, UserStorage, ViewStorage,
};
