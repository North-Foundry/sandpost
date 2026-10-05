//! Scope persistence and policy-version validation on SQLite.
use crate::SqliteStorage;
use crate::error::StorageResult;
use crate::records::{invalid_column, parse_identifier};
use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, params};
use sandpost_core::{Scope, ScopeIdentifier, UserIdentifier};
use sandpost_storage::{ScopeStorage, StorageError};

const SCOPE_COLUMNS: &str = "identifier,parent_identifier,name,description,filter,position,policy_version,endpoint_identifier";

fn decode_scope(row: &rusqlite::Row<'_>) -> rusqlite::Result<Scope> {
    let identifier: String = row.get(0)?;
    let parent: Option<String> = row.get(1)?;
    let endpoint: String = row.get(7)?;
    let policy_version: i64 = row.get(6)?;
    Ok(Scope {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column(0, "identifier"))?,
        parent: parent
            .map(|value| {
                parse_identifier(&value).map_err(|_| invalid_column(1, "parent_identifier"))
            })
            .transpose()?,
        name: row.get(2)?,
        description: row.get(3)?,
        filter: row.get(4)?,
        position: row.get(5)?,
        policy_version: u64::try_from(policy_version)
            .map_err(|_| invalid_column(6, "policy_version"))?,
        endpoint_identifier: parse_identifier(&endpoint)
            .map_err(|_| invalid_column(7, "endpoint_identifier"))?,
    })
}

/// Load one scope by identifier.
fn get_scope_blocking(
    connection: &Connection,
    identifier: ScopeIdentifier,
) -> Result<Option<Scope>, StorageError> {
    let query = format!("SELECT {SCOPE_COLUMNS} FROM scopes WHERE identifier=?1");
    connection
        .query_row(&query, [identifier.to_string()], decode_scope)
        .optional()
        .storage()
}

/// Load scopes ordered by their configured position and identifier.
fn list_scopes_blocking(connection: &Connection) -> Result<Vec<Scope>, StorageError> {
    let query = format!("SELECT {SCOPE_COLUMNS} FROM scopes ORDER BY position, identifier");
    let mut statement = connection.prepare(&query).storage()?;
    statement
        .query_map([], decode_scope)
        .storage()?
        .collect::<Result<Vec<_>, _>>()
        .storage()
}

/// Save a scope while enforcing policy-version changes for filter or parent updates.
fn save_scope_blocking(connection: &mut Connection, scope: &Scope) -> Result<(), StorageError> {
    let version = i64::try_from(scope.policy_version).map_err(|_| StorageError::IntegerRange)?;
    let transaction = connection.transaction().storage()?;
    let previous: Option<(String, Option<String>, i64, String)> = transaction
        .query_row(
            "SELECT filter, parent_identifier, policy_version, endpoint_identifier FROM scopes WHERE identifier=?1",
            [scope.identifier.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .storage()?;
    if let Some((old_filter, old_parent, old_version, old_endpoint)) = previous
        && (version < old_version
            || (version == old_version
                && (scope.filter != old_filter
                    || scope.parent.map(|identifier| identifier.to_string()) != old_parent
                    || scope.endpoint_identifier.to_string() != old_endpoint)))
    {
        return Err(StorageError::PolicyVersionConflict(scope.identifier));
    }
    transaction
        .execute(
            "INSERT INTO scopes(identifier, parent_identifier, name, description, filter, position, policy_version, endpoint_identifier) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT(identifier) DO UPDATE SET parent_identifier=excluded.parent_identifier, name=excluded.name, description=excluded.description, filter=excluded.filter, position=excluded.position, policy_version=excluded.policy_version, endpoint_identifier=excluded.endpoint_identifier",
            params![
                scope.identifier.to_string(),
                scope.parent.map(|identifier| identifier.to_string()),
                scope.name,
                scope.description,
                scope.filter,
                scope.position,
                version,
                scope.endpoint_identifier.to_string(),
            ],
        )
        .storage()?;
    transaction.commit().storage()?;
    Ok(())
}

/// Delete a scope when no child scope references it.
fn delete_scope_blocking(
    connection: &Connection,
    identifier: ScopeIdentifier,
) -> Result<bool, StorageError> {
    Ok(connection
        .execute(
            "DELETE FROM scopes WHERE identifier=?1",
            [identifier.to_string()],
        )
        .storage()?
        > 0)
}

/// Assign a user to a scope, enforcing the endpoint-membership invariant.
fn assign_user_to_scope_blocking(
    connection: &mut Connection,
    user: UserIdentifier,
    scope: ScopeIdentifier,
) -> Result<(), StorageError> {
    let transaction = connection.transaction().storage()?;
    let endpoint: Option<String> = transaction
        .query_row(
            "SELECT endpoint_identifier FROM scopes WHERE identifier=?1",
            [scope.to_string()],
            |row| row.get(0),
        )
        .optional()
        .storage()?;
    let endpoint = endpoint.ok_or(StorageError::NotFound)?;
    let has_membership: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM endpoint_memberships WHERE user_identifier=?1 AND endpoint_identifier=?2)",
            params![user.to_string(), endpoint],
            |row| row.get(0),
        )
        .storage()?;
    if !has_membership {
        return Err(StorageError::ConstraintViolation(
            "a scope membership requires an endpoint membership".into(),
        ));
    }
    transaction
        .execute(
            "INSERT INTO scope_memberships(user_identifier,scope_identifier) VALUES (?1,?2) ON CONFLICT(user_identifier,scope_identifier) DO NOTHING",
            params![user.to_string(), scope.to_string()],
        )
        .storage()?;
    transaction.commit().storage()?;
    Ok(())
}

/// Remove one user's assignment to a scope.
fn remove_user_from_scope_blocking(
    connection: &Connection,
    user: UserIdentifier,
    scope: ScopeIdentifier,
) -> Result<bool, StorageError> {
    Ok(connection
        .execute(
            "DELETE FROM scope_memberships WHERE user_identifier=?1 AND scope_identifier=?2",
            params![user.to_string(), scope.to_string()],
        )
        .storage()?
        > 0)
}

/// List all scope identifiers assigned to a user in deterministic order.
fn list_user_scopes_blocking(
    connection: &Connection,
    user: UserIdentifier,
) -> Result<Vec<ScopeIdentifier>, StorageError> {
    let mut statement = connection
        .prepare(
            "SELECT scope_identifier FROM scope_memberships WHERE user_identifier=?1 ORDER BY scope_identifier",
        )
        .storage()?;
    statement
        .query_map([user.to_string()], |row| row.get::<_, String>(0))
        .storage()?
        .map(|row| parse_identifier(&row.storage()?))
        .collect()
}

/// List users assigned to one scope in deterministic order.
fn list_scope_members_blocking(
    connection: &Connection,
    scope: ScopeIdentifier,
) -> Result<Vec<UserIdentifier>, StorageError> {
    let mut statement = connection
        .prepare(
            "SELECT user_identifier FROM scope_memberships WHERE scope_identifier=?1 ORDER BY user_identifier",
        )
        .storage()?;
    statement
        .query_map([scope.to_string()], |row| row.get::<_, String>(0))
        .storage()?
        .map(|row| parse_identifier(&row.storage()?))
        .collect()
}

#[async_trait]
impl ScopeStorage for SqliteStorage {
    async fn get_scope(&self, identifier: ScopeIdentifier) -> Result<Option<Scope>, StorageError> {
        self.run(move |connection| get_scope_blocking(connection, identifier))
            .await
    }

    async fn list_scopes(&self) -> Result<Vec<Scope>, StorageError> {
        self.run(|connection| list_scopes_blocking(connection))
            .await
    }

    async fn save_scope(&self, scope: &Scope) -> Result<(), StorageError> {
        let scope = scope.clone();
        self.run(move |connection| save_scope_blocking(connection, &scope))
            .await
    }

    async fn delete_scope(&self, identifier: ScopeIdentifier) -> Result<bool, StorageError> {
        self.run(move |connection| delete_scope_blocking(connection, identifier))
            .await
    }

    async fn assign_user_to_scope(
        &self,
        user: UserIdentifier,
        scope: ScopeIdentifier,
    ) -> Result<(), StorageError> {
        self.run(move |connection| assign_user_to_scope_blocking(connection, user, scope))
            .await
    }

    async fn remove_user_from_scope(
        &self,
        user: UserIdentifier,
        scope: ScopeIdentifier,
    ) -> Result<bool, StorageError> {
        self.run(move |connection| remove_user_from_scope_blocking(connection, user, scope))
            .await
    }

    async fn list_user_scopes(
        &self,
        user: UserIdentifier,
    ) -> Result<Vec<ScopeIdentifier>, StorageError> {
        self.run(move |connection| list_user_scopes_blocking(connection, user))
            .await
    }

    async fn list_scope_members(
        &self,
        scope: ScopeIdentifier,
    ) -> Result<Vec<UserIdentifier>, StorageError> {
        self.run(move |connection| list_scope_members_blocking(connection, scope))
            .await
    }
}
