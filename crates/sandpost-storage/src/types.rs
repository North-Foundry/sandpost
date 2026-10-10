//! Backend-independent request and result types used by the storage contract.

/// The backend-independent filter representation accepted by message queries.
///
/// This is the canonical SandPost query AST produced by the shared query language parser. A
/// backend compiles it into its own execution strategy but never receives raw SQL.
pub type FilterExpression = sandpost_query::Expression;

mod messages;
mod search;
mod smtp;
mod users;

pub use messages::{MessageListQuery, MessageSummary};
pub use search::{IndexedMessage, SearchOperation, SearchSynchronization};
pub use smtp::{NewSmtpAccess, SmtpAccessCredential};
pub use users::{NewUser, UpdateUser};
