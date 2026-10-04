//! Scope persistence and policy version validation.
use crate::{Storage, StorageError, records::parse_identifier};
use rusqlite::{OptionalExtension, params};
use sandpost_core::Scope;

impl Storage {
    /// Save a scope while enforcing policy-version changes for filter or parent updates.
    pub fn save_scope(&self, scope: &Scope) -> Result<(), StorageError> {
        let version =
            i64::try_from(scope.policy_version).map_err(|_| StorageError::IntegerRange)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let previous: Option<(String, Option<String>, i64)> = transaction
            .query_row(
                "SELECT filter, parent_identifier, policy_version FROM scopes WHERE identifier=?1",
                [scope.identifier.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((old_filter, old_parent, old_version)) = previous
            && (version < old_version
                || (version == old_version
                    && (scope.filter != old_filter
                        || scope.parent.map(|identifier| identifier.to_string()) != old_parent)))
        {
            return Err(StorageError::PolicyVersionConflict(scope.identifier));
        }
        transaction.execute(
            "INSERT INTO scopes(identifier, parent_identifier, name, description, filter, position, policy_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(identifier) DO UPDATE SET parent_identifier=excluded.parent_identifier, name=excluded.name, description=excluded.description, filter=excluded.filter, position=excluded.position, policy_version=excluded.policy_version",
            params![scope.identifier.to_string(), scope.parent.map(|identifier| identifier.to_string()), scope.name, scope.description, scope.filter, scope.position, version],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Load scopes ordered by their configured position and identifier.
    pub fn load_scopes(&self) -> Result<Vec<Scope>, StorageError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare_cached("SELECT identifier, parent_identifier, name, description, filter, position, policy_version FROM scopes ORDER BY position, identifier")?;
        let rows = statement.query_map([], |database_row| {
            Ok((
                database_row.get::<_, String>(0)?,
                database_row.get::<_, Option<String>>(1)?,
                database_row.get::<_, String>(2)?,
                database_row.get::<_, Option<String>>(3)?,
                database_row.get::<_, String>(4)?,
                database_row.get::<_, i64>(5)?,
                database_row.get::<_, i64>(6)?,
            ))
        })?;
        rows.map(|database_row| {
            let (identifier, parent, name, description, filter, position, policy_version) =
                database_row?;
            Ok(Scope {
                identifier: parse_identifier(&identifier)?,
                parent: parent
                    .map(|identifier| parse_identifier(&identifier))
                    .transpose()?,
                name,
                description,
                filter,
                position,
                policy_version: u64::try_from(policy_version)
                    .map_err(|_| StorageError::IntegerRange)?,
            })
        })
        .collect()
    }
}
