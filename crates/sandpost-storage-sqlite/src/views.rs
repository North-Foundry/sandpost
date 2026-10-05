//! Saved view persistence and the legacy recipient-inbox compatibility helper.
use crate::SqliteStorage;
use crate::error::StorageResult;
use crate::records::{invalid_column, parse_identifier};
use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sandpost_core::{UserIdentifier, View, ViewIdentifier, default_endpoint_identifier};
use sandpost_storage::{StorageError, ViewStorage};

const VIEW_COLUMNS: &str = "identifier,endpoint_identifier,owner_identifier,name,filter";

fn decode_view(row: &rusqlite::Row<'_>) -> rusqlite::Result<View> {
    let identifier: String = row.get(0)?;
    let endpoint: String = row.get(1)?;
    let owner: Option<String> = row.get(2)?;
    Ok(View {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column(0, "identifier"))?,
        endpoint_identifier: parse_identifier(&endpoint)
            .map_err(|_| invalid_column(1, "endpoint_identifier"))?,
        owner_identifier: owner
            .map(|value| {
                parse_identifier(&value).map_err(|_| invalid_column(2, "owner_identifier"))
            })
            .transpose()?,
        name: row.get(3)?,
        filter: row.get(4)?,
    })
}

/// List a user's personal views and all shared views by name and identifier.
fn list_views_blocking(
    connection: &Connection,
    user: UserIdentifier,
) -> Result<Vec<View>, StorageError> {
    let query = format!(
        "SELECT {VIEW_COLUMNS} FROM views WHERE owner_identifier=?1 OR owner_identifier IS NULL ORDER BY name,identifier"
    );
    let mut statement = connection.prepare(&query).storage()?;
    statement
        .query_map([user.to_string()], decode_view)
        .storage()?
        .collect::<Result<Vec<_>, _>>()
        .storage()
}

/// Load one view by identifier.
fn get_view_blocking(
    connection: &Connection,
    identifier: ViewIdentifier,
) -> Result<Option<View>, StorageError> {
    let query = format!("SELECT {VIEW_COLUMNS} FROM views WHERE identifier=?1");
    connection
        .query_row(&query, [identifier.to_string()], decode_view)
        .optional()
        .storage()
}

/// Insert or update a saved view.
fn save_view_blocking(connection: &Connection, view: &View) -> Result<(), StorageError> {
    connection
        .execute(
            "INSERT INTO views(identifier,endpoint_identifier,owner_identifier,name,filter) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(identifier) DO UPDATE SET endpoint_identifier=excluded.endpoint_identifier,owner_identifier=excluded.owner_identifier,name=excluded.name,filter=excluded.filter",
            params![
                view.identifier.to_string(),
                view.endpoint_identifier.to_string(),
                view.owner_identifier.map(|identifier| identifier.to_string()),
                view.name,
                view.filter,
            ],
        )
        .storage()?;
    Ok(())
}

/// Delete a saved view by identifier.
fn delete_view_blocking(
    connection: &Connection,
    identifier: ViewIdentifier,
) -> Result<bool, StorageError> {
    Ok(connection
        .execute(
            "DELETE FROM views WHERE identifier=?1",
            [identifier.to_string()],
        )
        .storage()?
        > 0)
}

/// Create a caller-owned recipient view with transactional duplicate-address protection.
fn create_recipient_view_blocking(
    connection: &mut Connection,
    user: UserIdentifier,
    name: &str,
    address: &str,
) -> Result<View, StorageError> {
    if !valid_local_address(address) {
        return Err(StorageError::InvalidInboxAddress);
    }
    let filter = format!("envelope.to.address == \"{address}\"");
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    let duplicate: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM views WHERE owner_identifier=?1 AND endpoint_identifier=?2 AND filter=?3)",
            params![
                user.to_string(),
                default_endpoint_identifier().to_string(),
                filter
            ],
            |row| row.get(0),
        )
        .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateInboxAddress);
    }
    let view = View {
        identifier: ViewIdentifier::new(),
        endpoint_identifier: default_endpoint_identifier(),
        owner_identifier: Some(user),
        name: name.to_owned(),
        filter,
    };
    transaction
        .execute(
            "INSERT INTO views(identifier,endpoint_identifier,owner_identifier,name,filter) VALUES (?1,?2,?3,?4,?5)",
            params![
                view.identifier.to_string(),
                view.endpoint_identifier.to_string(),
                user.to_string(),
                view.name,
                view.filter,
            ],
        )
        .storage()?;
    transaction.commit().storage()?;
    Ok(view)
}

/// Accept only normalized local addresses that cannot alter the generated DSL string.
pub(crate) fn valid_local_address(address: &str) -> bool {
    let Some((local_part, domain)) = address.split_once('@') else {
        return false;
    };
    domain == "sandpost.local"
        && address == address.to_ascii_lowercase()
        && !local_part.is_empty()
        && local_part.len() <= 64
        && !local_part.starts_with('.')
        && !local_part.ends_with('.')
        && local_part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[async_trait]
impl ViewStorage for SqliteStorage {
    async fn list_views(&self, user: UserIdentifier) -> Result<Vec<View>, StorageError> {
        self.run(move |connection| list_views_blocking(connection, user))
            .await
    }

    async fn get_view(&self, identifier: ViewIdentifier) -> Result<Option<View>, StorageError> {
        self.run(move |connection| get_view_blocking(connection, identifier))
            .await
    }

    async fn save_view(&self, view: &View) -> Result<(), StorageError> {
        let view = view.clone();
        self.run(move |connection| save_view_blocking(connection, &view))
            .await
    }

    async fn delete_view(&self, identifier: ViewIdentifier) -> Result<bool, StorageError> {
        self.run(move |connection| delete_view_blocking(connection, identifier))
            .await
    }

    async fn create_recipient_view(
        &self,
        user: UserIdentifier,
        name: &str,
        address: &str,
    ) -> Result<View, StorageError> {
        let name = name.to_owned();
        let address = address.to_owned();
        self.run(move |connection| {
            create_recipient_view_blocking(connection, user, &name, &address)
        })
        .await
    }
}
