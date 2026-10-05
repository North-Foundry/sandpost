#![doc = include_str!("../README.md")]

mod connection;
mod endpoints;
mod error;
mod filter;
mod mail_parts;
mod messages;
mod migrations;
mod records;
mod schema;
mod scopes;
mod search_sync;
mod users;
mod views;

pub use connection::SqliteStorage;
pub use sandpost_storage::{
    EndpointStorage, MessageStorage, ScopeStorage, SearchSynchronizationStorage, Storage,
    StorageError, StorageHealth, UserStorage, ViewStorage,
};
