//! Shared link check results of physical messages on SQLite.
use crate::SqliteStorage;
use crate::error::StorageResult;
use crate::records::invalid_column;
use async_trait::async_trait;
use sandpost_core::{
    LinkCheckErrorKind, LinkCheckResult, LinkOutcome, MessageIdentifier, MessageLinkCheck,
};
use sandpost_storage::{LinkCheckStorage, StorageError};
use sqlx::{QueryBuilder, Row, Sqlite, sqlite::SqliteRow};

/// Result rows written per multi-row statement, well below SQLite's bound parameter limit.
const RESULT_ROWS_PER_STATEMENT: usize = 128;

/**
 * Decode one stored link result row.
 */
fn decode_result(row: SqliteRow) -> Result<LinkCheckResult, sqlx::Error> {
    let outcome: String = row.try_get("outcome")?;
    let status_code: Option<i64> = row.try_get("status_code")?;
    let redirect_count: i64 = row.try_get("redirect_count")?;
    let error_kind: Option<String> = row.try_get("error_kind")?;
    Ok(LinkCheckResult {
        url: row.try_get("url")?,
        outcome: LinkOutcome::parse(&outcome).ok_or_else(|| invalid_column("outcome"))?,
        status_code: status_code
            .map(|code| u16::try_from(code).map_err(|_| invalid_column("status_code")))
            .transpose()?,
        final_url: row.try_get("final_url")?,
        redirect_count: u32::try_from(redirect_count)
            .map_err(|_| invalid_column("redirect_count"))?,
        error_kind: error_kind
            .map(|kind| {
                LinkCheckErrorKind::parse(&kind).ok_or_else(|| invalid_column("error_kind"))
            })
            .transpose()?,
        error_message: row.try_get("error_message")?,
        checked_at: row.try_get("checked_at")?,
    })
}

/**
 * Read a message's stored check and its ordered results from one read snapshot.
 */
async fn get_message_link_check(
    storage: &SqliteStorage,
    message: MessageIdentifier,
) -> Result<Option<MessageLinkCheck>, StorageError> {
    let mut transaction = storage.read_pool.begin().await.storage()?;
    let Some(row) = sqlx::query(
        "SELECT checked_at,omitted_link_count FROM message_link_checks WHERE message_identifier=?1",
    )
    .bind(message.to_string())
    .fetch_optional(&mut *transaction)
    .await
    .storage()?
    else {
        return Ok(None);
    };
    let checked_at: i64 = row.try_get("checked_at").storage()?;
    let omitted_link_count: i64 = row.try_get("omitted_link_count").storage()?;
    let links = sqlx::query(
        "SELECT url,outcome,status_code,final_url,redirect_count,error_kind,error_message,checked_at FROM message_link_check_results WHERE message_identifier=?1 ORDER BY position",
    )
    .bind(message.to_string())
    .fetch_all(&mut *transaction)
    .await
    .storage()?
    .into_iter()
    .map(decode_result)
    .collect::<Result<Vec<_>, _>>()
    .storage()?;
    transaction.commit().await.storage()?;
    Ok(Some(MessageLinkCheck {
        message_identifier: message,
        checked_at,
        omitted_link_count: u32::try_from(omitted_link_count)
            .map_err(|_| StorageError::InvalidData("invalid omitted link count".into()))?,
        links,
    }))
}

/**
 * Replace a message's stored check unless a newer check is already stored.
 */
async fn replace_message_link_check(
    storage: &SqliteStorage,
    check: &MessageLinkCheck,
) -> Result<bool, StorageError> {
    let message = check.message_identifier.to_string();
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM mail WHERE identifier=?1)")
        .bind(&message)
        .fetch_one(&mut *transaction)
        .await
        .storage()?;
    if !exists {
        return Err(StorageError::NotFound);
    }
    let stored: Option<i64> = sqlx::query_scalar(
        "SELECT checked_at FROM message_link_checks WHERE message_identifier=?1",
    )
    .bind(&message)
    .fetch_optional(&mut *transaction)
    .await
    .storage()?;
    if stored.is_some_and(|stored| stored > check.checked_at) {
        return Ok(false);
    }
    // Deleting the check cascades its previous results inside this transaction.
    sqlx::query("DELETE FROM message_link_checks WHERE message_identifier=?1")
        .bind(&message)
        .execute(&mut *transaction)
        .await
        .storage()?;
    sqlx::query(
        "INSERT INTO message_link_checks(message_identifier,checked_at,omitted_link_count) VALUES (?1,?2,?3)",
    )
    .bind(&message)
    .bind(check.checked_at)
    .bind(i64::from(check.omitted_link_count))
    .execute(&mut *transaction)
    .await
    .storage()?;
    for (chunk_index, chunk) in check.links.chunks(RESULT_ROWS_PER_STATEMENT).enumerate() {
        let mut statement = QueryBuilder::<Sqlite>::new(
            "INSERT INTO message_link_check_results(message_identifier,url,position,outcome,status_code,final_url,redirect_count,error_kind,error_message,checked_at) ",
        );
        let first_position = chunk_index * RESULT_ROWS_PER_STATEMENT;
        let mut positions = Vec::with_capacity(chunk.len());
        for offset in 0..chunk.len() {
            positions.push(
                i64::try_from(first_position + offset).map_err(|_| StorageError::IntegerRange)?,
            );
        }
        statement.push_values(
            chunk.iter().zip(positions),
            |mut row, (result, position)| {
                row.push_bind(&message)
                    .push_bind(&result.url)
                    .push_bind(position)
                    .push_bind(result.outcome.as_str())
                    .push_bind(result.status_code.map(i64::from))
                    .push_bind(&result.final_url)
                    .push_bind(i64::from(result.redirect_count))
                    .push_bind(result.error_kind.map(LinkCheckErrorKind::as_str))
                    .push_bind(&result.error_message)
                    .push_bind(result.checked_at);
            },
        );
        statement
            .build()
            .execute(&mut *transaction)
            .await
            .storage()?;
    }
    transaction.commit().await.storage()?;
    Ok(true)
}

#[async_trait]
impl LinkCheckStorage for SqliteStorage {
    /**
     * Load the stored check of a message with its results in message order.
     */
    async fn get_message_link_check(
        &self,
        message: MessageIdentifier,
    ) -> Result<Option<MessageLinkCheck>, StorageError> {
        get_message_link_check(self, message).await
    }

    /**
     * Atomically replace the stored check of a message, ignoring an older replacement.
     */
    async fn replace_message_link_check(
        &self,
        check: &MessageLinkCheck,
    ) -> Result<bool, StorageError> {
        replace_message_link_check(self, check).await
    }
}
