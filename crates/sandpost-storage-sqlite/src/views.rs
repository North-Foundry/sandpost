//! Saved view persistence and the legacy recipient-inbox compatibility helper.
use crate::SqliteStorage;
use crate::error::StorageResult;
use crate::records::{invalid_column, parse_identifier};
use async_trait::async_trait;
use sandpost_core::{UserIdentifier, View, ViewIdentifier};
use sandpost_storage::{StorageError, ViewStorage};
use sqlx::{Row, SqliteConnection, sqlite::SqliteRow};

const VIEW_COLUMNS: &str = "identifier,owner_identifier,name,filter";

/// Decode a saved view row, validating its identifier and optional owner.
fn decode_view(row: SqliteRow) -> Result<View, sqlx::Error> {
    let identifier: String = row.try_get("identifier")?;
    let owner: Option<String> = row.try_get("owner_identifier")?;
    Ok(View {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column("identifier"))?,
        owner_identifier: owner
            .map(|value| parse_identifier(&value).map_err(|_| invalid_column("owner_identifier")))
            .transpose()?,
        name: row.try_get("name")?,
        filter: row.try_get("filter")?,
    })
}

/// List a user's personal views and all shared views by name and identifier.
async fn list_views(
    connection: &mut SqliteConnection,
    user: UserIdentifier,
) -> Result<Vec<View>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {VIEW_COLUMNS} FROM views WHERE owner_identifier=?1 OR owner_identifier IS NULL ORDER BY name,identifier"
    )))
    .bind(user.to_string())
    .fetch_all(connection)
    .await
    .storage()?
    .into_iter()
    .map(decode_view)
    .collect::<Result<Vec<_>, _>>()
    .storage()
}

/// Load one view by identifier.
async fn get_view(
    connection: &mut SqliteConnection,
    identifier: ViewIdentifier,
) -> Result<Option<View>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {VIEW_COLUMNS} FROM views WHERE identifier=?1"
    )))
    .bind(identifier.to_string())
    .fetch_optional(connection)
    .await
    .storage()?
    .map(decode_view)
    .transpose()
    .storage()
}

/// Insert or update a saved view.
async fn save_view(storage: &SqliteStorage, view: &View) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO views(identifier,owner_identifier,name,filter) VALUES (?1,?2,?3,?4) ON CONFLICT(identifier) DO UPDATE SET owner_identifier=excluded.owner_identifier,name=excluded.name,filter=excluded.filter",
    )
    .bind(view.identifier.to_string())
    .bind(view.owner_identifier.map(|identifier| identifier.to_string()))
    .bind(&view.name)
    .bind(&view.filter)
    .execute(&storage.write_pool)
    .await
    .storage()?;
    Ok(())
}

/// Delete a saved view by identifier.
async fn delete_view(
    storage: &SqliteStorage,
    identifier: ViewIdentifier,
) -> Result<bool, StorageError> {
    Ok(sqlx::query("DELETE FROM views WHERE identifier=?1")
        .bind(identifier.to_string())
        .execute(&storage.write_pool)
        .await
        .storage()?
        .rows_affected()
        > 0)
}

/// Create a caller-owned recipient view with transactional duplicate-address protection.
async fn create_recipient_view(
    storage: &SqliteStorage,
    user: UserIdentifier,
    name: &str,
    address: &str,
) -> Result<View, StorageError> {
    if !valid_local_address(address) {
        return Err(StorageError::InvalidInboxAddress);
    }
    let filter = format!("envelope.to.address == \"{address}\"");
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let duplicate: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM views WHERE owner_identifier=?1 AND filter=?2)",
    )
    .bind(user.to_string())
    .bind(&filter)
    .fetch_one(&mut *transaction)
    .await
    .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateInboxAddress);
    }
    let view = View {
        identifier: ViewIdentifier::new(),
        owner_identifier: Some(user),
        name: name.to_owned(),
        filter,
    };
    sqlx::query("INSERT INTO views(identifier,owner_identifier,name,filter) VALUES (?1,?2,?3,?4)")
        .bind(view.identifier.to_string())
        .bind(user.to_string())
        .bind(&view.name)
        .bind(&view.filter)
        .execute(&mut *transaction)
        .await
        .storage()?;
    transaction.commit().await.storage()?;
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
    /// List the caller-owned and shared saved views in name and identifier order.
    async fn list_views(&self, user: UserIdentifier) -> Result<Vec<View>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        list_views(&mut connection, user).await
    }

    /// Read a saved view by identifier.
    async fn get_view(&self, identifier: ViewIdentifier) -> Result<Option<View>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        get_view(&mut connection, identifier).await
    }

    /// Insert or update one saved view and its canonical filter.
    async fn save_view(&self, view: &View) -> Result<(), StorageError> {
        save_view(self, view).await
    }

    /// Delete a saved view and its dependent IMAP configuration.
    async fn delete_view(&self, identifier: ViewIdentifier) -> Result<bool, StorageError> {
        delete_view(self, identifier).await
    }

    /// Create a personal view for a validated local address with duplicate protection.
    async fn create_recipient_view(
        &self,
        user: UserIdentifier,
        name: &str,
        address: &str,
    ) -> Result<View, StorageError> {
        create_recipient_view(self, user, name, address).await
    }
}
