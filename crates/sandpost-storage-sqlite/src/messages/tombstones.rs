//! Per-user message deletion state.
use crate::error::StorageResult;
use crate::filter::{SqlValue, bind_query, compile};
use sandpost_core::{MessageIdentifier, UserIdentifier};
use sandpost_storage::{FilterExpression, StorageError};
use sqlx::SqliteConnection;

/// Maximum number of identifiers accepted by one explicit user deletion request.
const MAXIMUM_USER_DELETION_BATCH_SIZE: usize = 256;

/// Insert tombstones for authorized filter matches and report newly inserted rows.
pub(super) async fn delete_user_messages(
    connection: &mut SqliteConnection,
    user: UserIdentifier,
    identifiers: Option<&[MessageIdentifier]>,
    filter: &FilterExpression,
) -> Result<u64, StorageError> {
    let user_exists: i64 =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE identifier = ?)")
            .bind(user.to_string())
            .fetch_one(&mut *connection)
            .await
            .storage()?;
    if user_exists == 0 {
        return Err(StorageError::NotFound);
    }
    if let Some(identifiers) = identifiers {
        if identifiers.len() > MAXIMUM_USER_DELETION_BATCH_SIZE {
            return Err(StorageError::BatchLimitExceeded(
                MAXIMUM_USER_DELETION_BATCH_SIZE,
            ));
        }
        if identifiers.is_empty() {
            return Ok(0);
        }
    }

    let compiled = compile(filter);
    let mut parameters = Vec::with_capacity(
        compiled.parameters.len() + 1 + identifiers.map_or(0, |identifiers| identifiers.len()),
    );
    parameters.push(SqlValue::Text(user.to_string()));
    parameters.extend(compiled.parameters);
    let identifier_clause = if let Some(identifiers) = identifiers {
        parameters.extend(
            identifiers
                .iter()
                .map(|identifier| SqlValue::Text(identifier.to_string())),
        );
        format!(
            " AND mail.identifier IN ({})",
            vec!["?"; identifiers.len()].join(",")
        )
    } else {
        String::new()
    };
    let sql = format!(
        "INSERT OR IGNORE INTO user_message_deletions(user_identifier, mail_sequence) SELECT ?, mail.sequence FROM mail WHERE ({}){}",
        compiled.sql, identifier_clause
    );
    let result = bind_query(sqlx::query(sqlx::AssertSqlSafe(sql)), &parameters)
        .execute(&mut *connection)
        .await
        .storage()?;
    Ok(result.rows_affected())
}
