//! Durable search-outbox synchronization state on SQLite.
//!
//! Other backends provide equivalent semantics: a committed message mutation and its outbox
//! operation are atomic, operations are replayable, and acknowledgement prunes an inclusive prefix.
use crate::SqliteStorage;
use crate::error::StorageResult;
use crate::records::parse_identifier;
use async_trait::async_trait;
use sandpost_storage::{
    SearchOperation, SearchSynchronization, SearchSynchronizationStorage, StorageError,
};
use sqlx::{Row, SqliteConnection};

/// Maximum number of pending search operations returned in one batch.
const MAXIMUM_SEARCH_OPERATION_BATCH_SIZE: usize = 64;

/// Decode durable synchronization state within the caller's connection or read snapshot.
async fn read_search_status(
    connection: &mut SqliteConnection,
) -> Result<SearchSynchronization, StorageError> {
    let row = sqlx::query(
        "SELECT latest_sequence, indexed_sequence FROM search_outbox_state WHERE singleton=1",
    )
    .fetch_one(&mut *connection)
    .await
    .storage()?;
    let latest_sequence = u64::try_from(row.try_get::<i64, _>("latest_sequence").storage()?)
        .map_err(|_| StorageError::IntegerRange)?;
    let indexed_sequence = u64::try_from(row.try_get::<i64, _>("indexed_sequence").storage()?)
        .map_err(|_| StorageError::IntegerRange)?;
    Ok(SearchSynchronization {
        latest_sequence,
        indexed_sequence,
        // Triggers append contiguous revisions; acknowledgement prunes an inclusive prefix.
        pending_operations: latest_sequence
            .checked_sub(indexed_sequence)
            .ok_or(StorageError::IntegerRange)?,
    })
}

/// Decode at most 64 pending operations after an exclusive cursor.
async fn read_search_operations(
    connection: &mut SqliteConnection,
    after: u64,
    limit: usize,
) -> Result<Vec<SearchOperation>, StorageError> {
    let after = i64::try_from(after).map_err(|_| StorageError::IntegerRange)?;
    let limit = i64::try_from(limit.min(MAXIMUM_SEARCH_OPERATION_BATCH_SIZE))
        .map_err(|_| StorageError::IntegerRange)?;
    let rows = sqlx::query(
        "SELECT sequence, message_identifier FROM search_outbox WHERE sequence>? ORDER BY sequence LIMIT ?",
    )
    .bind(after)
    .bind(limit)
    .fetch_all(&mut *connection)
    .await
    .storage()?;
    rows.into_iter()
        .map(|row| {
            Ok(SearchOperation {
                sequence: u64::try_from(row.try_get::<i64, _>("sequence").storage()?)
                    .map_err(|_| StorageError::IntegerRange)?,
                message_identifier: parse_identifier(
                    &row.try_get::<String, _>("message_identifier").storage()?,
                )?,
            })
        })
        .collect()
}

/// Read state and a bounded pending batch from one consistent SQL snapshot.
async fn search_batch(
    connection: &mut SqliteConnection,
    after: u64,
    limit: usize,
) -> Result<(SearchSynchronization, Vec<SearchOperation>), StorageError> {
    let status = read_search_status(connection).await?;
    let operations = read_search_operations(connection, after, limit).await?;
    Ok((status, operations))
}

/// Advance the acknowledged watermark and prune its inclusive prefix in one transaction.
async fn acknowledge_search_operations(
    connection: &mut SqliteConnection,
    through: u64,
) -> Result<(), StorageError> {
    let through = i64::try_from(through).map_err(|_| StorageError::IntegerRange)?;
    sqlx::query(
        "UPDATE search_outbox_state SET indexed_sequence=MAX(indexed_sequence,MIN(latest_sequence,?)) WHERE singleton=1",
    )
    .bind(through)
    .execute(&mut *connection)
    .await
    .storage()?;
    sqlx::query(
        "DELETE FROM search_outbox WHERE sequence<=(SELECT indexed_sequence FROM search_outbox_state WHERE singleton=1)",
    )
    .execute(&mut *connection)
    .await
    .storage()?;
    Ok(())
}

#[async_trait]
impl SearchSynchronizationStorage for SqliteStorage {
    /// Read the durable latest and acknowledged revision watermarks.
    async fn search_status(&self) -> Result<SearchSynchronization, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        read_search_status(&mut connection).await
    }

    /// Read synchronization state and pending operations from one SQL snapshot.
    async fn search_batch(
        &self,
        after: u64,
        limit: usize,
    ) -> Result<(SearchSynchronization, Vec<SearchOperation>), StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let batch = search_batch(&mut transaction, after, limit).await?;
        transaction.commit().await.storage()?;
        Ok(batch)
    }

    /// Read a bounded ordered batch after an exclusive synchronization cursor.
    async fn pending_search_operations(
        &self,
        after: u64,
        limit: usize,
    ) -> Result<Vec<SearchOperation>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        read_search_operations(&mut connection, after, limit).await
    }

    /// Advance the acknowledged watermark and prune its inclusive prefix atomically.
    async fn acknowledge_search_operations(&self, through: u64) -> Result<(), StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        acknowledge_search_operations(&mut transaction, through).await?;
        transaction.commit().await.storage()?;
        Ok(())
    }
}
