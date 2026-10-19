//! Storage capabilities grouped by domain.

mod health;
mod link_checks;
mod messages;
mod scopes;
mod search;
mod smtp_server;
mod storage;
mod users;
mod views;

pub use health::StorageHealth;
pub use link_checks::LinkCheckStorage;
pub use messages::MessageStorage;
pub use scopes::ScopeStorage;
pub use search::SearchSynchronizationStorage;
pub use smtp_server::SmtpServerStorage;
pub use storage::Storage;
pub use users::UserStorage;
pub use views::ViewStorage;
