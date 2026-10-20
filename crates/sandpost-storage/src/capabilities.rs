//! Storage capabilities grouped by domain.

mod health;
mod link_checks;
mod messages;
mod personas;
mod scopes;
mod search;
mod smtp_server;
mod storage;
mod users;
mod views;

pub use health::StorageHealth;
pub use link_checks::LinkCheckStorage;
pub use messages::MessageStorage;
pub use personas::{PERSONA_EXECUTION_RETENTION, PERSONA_PENDING_EXECUTION_LIMIT, PersonaStorage};
pub use scopes::ScopeStorage;
pub use search::SearchSynchronizationStorage;
pub use smtp_server::SmtpServerStorage;
pub use storage::Storage;
pub use users::UserStorage;
pub use views::ViewStorage;
