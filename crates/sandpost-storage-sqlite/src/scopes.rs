//! Scope persistence and policy-version validation on SQLite.
use crate::SqliteStorage;
use crate::error::StorageResult;
use crate::records::{invalid_column, parse_identifier};
use async_trait::async_trait;
use sandpost_core::{Scope, ScopeIdentifier, UserIdentifier};
use sandpost_storage::{ScopeStorage, StorageError};
use sqlx::{Row, SqliteConnection, sqlite::SqliteRow};

const SCOPE_COLUMNS: &str =
    "identifier,parent_identifier,name,description,filter,position,policy_version";

/// Decode a scope row, validating identifiers and its unsigned policy version.
fn decode_scope(row: SqliteRow) -> Result<Scope, sqlx::Error> {
    let identifier: String = row.try_get("identifier")?;
    let parent: Option<String> = row.try_get("parent_identifier")?;
    let policy_version: i64 = row.try_get("policy_version")?;
    Ok(Scope {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column("identifier"))?,
        parent: parent
            .map(|value| parse_identifier(&value).map_err(|_| invalid_column("parent_identifier")))
            .transpose()?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        filter: row.try_get("filter")?,
        position: row.try_get("position")?,
        policy_version: u64::try_from(policy_version)
            .map_err(|_| invalid_column("policy_version"))?,
    })
}

/// Load one scope by identifier.
async fn get_scope(
    connection: &mut SqliteConnection,
    identifier: ScopeIdentifier,
) -> Result<Option<Scope>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {SCOPE_COLUMNS} FROM scopes WHERE identifier=?1"
    )))
    .bind(identifier.to_string())
    .fetch_optional(connection)
    .await
    .storage()?
    .map(decode_scope)
    .transpose()
    .storage()
}

/// Load scopes ordered by their configured position and identifier.
async fn list_scopes(connection: &mut SqliteConnection) -> Result<Vec<Scope>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {SCOPE_COLUMNS} FROM scopes ORDER BY position, identifier"
    )))
    .fetch_all(connection)
    .await
    .storage()?
    .into_iter()
    .map(decode_scope)
    .collect::<Result<Vec<_>, _>>()
    .storage()
}

/// Save a scope while enforcing policy-version changes for filter or parent updates.
async fn save_scope(storage: &SqliteStorage, scope: &Scope) -> Result<(), StorageError> {
    let version = i64::try_from(scope.policy_version).map_err(|_| StorageError::IntegerRange)?;
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let previous: Option<(String, Option<String>, i64)> = sqlx::query(
        "SELECT filter, parent_identifier, policy_version FROM scopes WHERE identifier=?1",
    )
    .bind(scope.identifier.to_string())
    .fetch_optional(&mut *transaction)
    .await
    .storage()?
    .map(|row| {
        Ok((
            row.try_get("filter")?,
            row.try_get("parent_identifier")?,
            row.try_get("policy_version")?,
        ))
    })
    .transpose()
    .storage()?;
    if let Some((old_filter, old_parent, old_version)) = previous
        && (version < old_version
            || (version == old_version
                && (scope.filter != old_filter
                    || scope.parent.map(|identifier| identifier.to_string()) != old_parent)))
    {
        return Err(StorageError::PolicyVersionConflict(scope.identifier));
    }
    sqlx::query(
        "INSERT INTO scopes(identifier, parent_identifier, name, description, filter, position, policy_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(identifier) DO UPDATE SET parent_identifier=excluded.parent_identifier, name=excluded.name, description=excluded.description, filter=excluded.filter, position=excluded.position, policy_version=excluded.policy_version",
    )
    .bind(scope.identifier.to_string())
    .bind(scope.parent.map(|identifier| identifier.to_string()))
    .bind(&scope.name)
    .bind(&scope.description)
    .bind(&scope.filter)
    .bind(scope.position)
    .bind(version)
    .execute(&mut *transaction)
    .await
    .storage()?;
    transaction.commit().await.storage()?;
    Ok(())
}

/// Delete a scope when no child scope references it.
async fn delete_scope(
    connection: &mut SqliteConnection,
    identifier: ScopeIdentifier,
) -> Result<bool, StorageError> {
    Ok(sqlx::query("DELETE FROM scopes WHERE identifier=?1")
        .bind(identifier.to_string())
        .execute(connection)
        .await
        .storage()?
        .rows_affected()
        > 0)
}

/// Assign a user to a scope: an unknown scope is NotFound and the foreign key rejects an unknown user.
async fn assign_user_to_scope(
    storage: &SqliteStorage,
    user: UserIdentifier,
    scope: ScopeIdentifier,
) -> Result<(), StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let scope_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM scopes WHERE identifier=?1)")
            .bind(scope.to_string())
            .fetch_one(&mut *transaction)
            .await
            .storage()?;
    if !scope_exists {
        return Err(StorageError::NotFound);
    }
    sqlx::query(
        "INSERT INTO scope_memberships(user_identifier,scope_identifier) VALUES (?1,?2) ON CONFLICT(user_identifier,scope_identifier) DO NOTHING",
    )
    .bind(user.to_string())
    .bind(scope.to_string())
    .execute(&mut *transaction)
    .await
    .storage()?;
    transaction.commit().await.storage()?;
    Ok(())
}

/// Remove one user's assignment to a scope.
async fn remove_user_from_scope(
    connection: &mut SqliteConnection,
    user: UserIdentifier,
    scope: ScopeIdentifier,
) -> Result<bool, StorageError> {
    Ok(sqlx::query(
        "DELETE FROM scope_memberships WHERE user_identifier=?1 AND scope_identifier=?2",
    )
    .bind(user.to_string())
    .bind(scope.to_string())
    .execute(connection)
    .await
    .storage()?
    .rows_affected()
        > 0)
}

/// List all scope identifiers assigned to a user in deterministic order.
async fn list_user_scopes(
    connection: &mut SqliteConnection,
    user: UserIdentifier,
) -> Result<Vec<ScopeIdentifier>, StorageError> {
    sqlx::query("SELECT scope_identifier FROM scope_memberships WHERE user_identifier=?1 ORDER BY scope_identifier")
        .bind(user.to_string())
        .fetch_all(connection)
        .await
        .storage()?
        .into_iter()
        .map(|row| {
            let value: String = row.try_get("scope_identifier").storage()?;
            parse_identifier(&value).map_err(|_| invalid_column("scope_identifier")).storage()
        })
        .collect()
}

/// List users assigned to one scope in deterministic order.
async fn list_scope_members(
    connection: &mut SqliteConnection,
    scope: ScopeIdentifier,
) -> Result<Vec<UserIdentifier>, StorageError> {
    sqlx::query("SELECT user_identifier FROM scope_memberships WHERE scope_identifier=?1 ORDER BY user_identifier")
        .bind(scope.to_string())
        .fetch_all(connection)
        .await
        .storage()?
        .into_iter()
        .map(|row| {
            let value: String = row.try_get("user_identifier").storage()?;
            parse_identifier(&value).map_err(|_| invalid_column("user_identifier")).storage()
        })
        .collect()
}

#[async_trait]
impl ScopeStorage for SqliteStorage {
    /// Read one scope by identifier.
    async fn get_scope(&self, identifier: ScopeIdentifier) -> Result<Option<Scope>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        get_scope(&mut connection, identifier).await
    }

    /// List scopes by their configured position and identifier.
    async fn list_scopes(&self) -> Result<Vec<Scope>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        list_scopes(&mut connection).await
    }

    /// Save a scope while enforcing monotonic policy versions for filter and parent changes.
    async fn save_scope(&self, scope: &Scope) -> Result<(), StorageError> {
        save_scope(self, scope).await
    }

    /// Delete a scope and its memberships when no child references it.
    async fn delete_scope(&self, identifier: ScopeIdentifier) -> Result<bool, StorageError> {
        let mut connection = self.write_pool.acquire().await.storage()?;
        delete_scope(&mut connection, identifier).await
    }

    /// Assign a user to an existing scope without duplicating its membership.
    async fn assign_user_to_scope(
        &self,
        user: UserIdentifier,
        scope: ScopeIdentifier,
    ) -> Result<(), StorageError> {
        assign_user_to_scope(self, user, scope).await
    }

    /// Remove one scope membership, reporting whether it existed.
    async fn remove_user_from_scope(
        &self,
        user: UserIdentifier,
        scope: ScopeIdentifier,
    ) -> Result<bool, StorageError> {
        let mut connection = self.write_pool.acquire().await.storage()?;
        remove_user_from_scope(&mut connection, user, scope).await
    }

    /// List assigned scope identifiers for one user in deterministic order.
    async fn list_user_scopes(
        &self,
        user: UserIdentifier,
    ) -> Result<Vec<ScopeIdentifier>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        list_user_scopes(&mut connection, user).await
    }

    /// List user identifiers assigned to a scope in deterministic order.
    async fn list_scope_members(
        &self,
        scope: ScopeIdentifier,
    ) -> Result<Vec<UserIdentifier>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        list_scope_members(&mut connection, scope).await
    }
}
