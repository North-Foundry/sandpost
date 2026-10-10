//! Durable search-outbox synchronization state on SQLite.
//!
//! Other backends must provide equivalent observable semantics: a committed message mutation and
//! its search-outbox operation are atomic, operations are replayable, and acknowledgement prunes
//! an inclusive prefix. SQLite satisfies that with transactional triggers.
use crate::SqliteStorage;
use crate::error::StorageResult;
use crate::records::parse_identifier;
use async_trait::async_trait;
use rusqlite::{Connection, params};
use sandpost_storage::{
    SearchOperation, SearchSynchronization, SearchSynchronizationStorage, StorageError,
};

const MAXIMUM_SEARCH_OPERATION_BATCH_SIZE: usize = 64;

/// Decode durable synchronization state within the caller's connection or read snapshot.
fn read_search_status(connection: &Connection) -> Result<SearchSynchronization, StorageError> {
    let (latest, indexed): (i64, i64) = connection
        .query_row(
            "SELECT latest_sequence,indexed_sequence FROM search_outbox_state WHERE singleton=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .storage()?;
    let latest_sequence = u64::try_from(latest).map_err(|_| StorageError::IntegerRange)?;
    let indexed_sequence = u64::try_from(indexed).map_err(|_| StorageError::IntegerRange)?;
    Ok(SearchSynchronization {
        latest_sequence,
        indexed_sequence,
        // Triggers append contiguous revisions; acknowledgement prunes an inclusive prefix.
        pending_operations: latest_sequence
            .checked_sub(indexed_sequence)
            .ok_or(StorageError::IntegerRange)?,
    })
}

/// Decode at most 64 pending operations after an exclusive cursor in the caller's snapshot.
fn read_search_operations(
    connection: &Connection,
    after: u64,
    limit: usize,
) -> Result<Vec<SearchOperation>, StorageError> {
    let after = i64::try_from(after).map_err(|_| StorageError::IntegerRange)?;
    let limit = i64::try_from(limit.min(MAXIMUM_SEARCH_OPERATION_BATCH_SIZE))
        .map_err(|_| StorageError::IntegerRange)?;
    let mut statement = connection
        .prepare(
            "SELECT sequence,message_identifier FROM search_outbox WHERE sequence>?1 ORDER BY sequence LIMIT ?2",
        )
        .storage()?;
    statement
        .query_map(params![after, limit], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .storage()?
        .map(|row| {
            let (sequence, identifier) = row.storage()?;
            Ok(SearchOperation {
                sequence: u64::try_from(sequence).map_err(|_| StorageError::IntegerRange)?,
                message_identifier: parse_identifier(&identifier)?,
            })
        })
        .collect()
}

/// Read synchronization state and a bounded pending batch from one SQL snapshot.
fn search_batch_blocking(
    connection: &mut Connection,
    after: u64,
    limit: usize,
) -> Result<(SearchSynchronization, Vec<SearchOperation>), StorageError> {
    let transaction = connection.transaction().storage()?;
    let status = read_search_status(&transaction)?;
    let operations = read_search_operations(&transaction, after, limit)?;
    transaction.commit().storage()?;
    Ok((status, operations))
}

/// Acknowledge all queued operations through an inclusive sequence in one transaction.
fn acknowledge_search_operations_blocking(
    connection: &mut Connection,
    through: u64,
) -> Result<(), StorageError> {
    let through = i64::try_from(through).map_err(|_| StorageError::IntegerRange)?;
    let transaction = connection.transaction().storage()?;
    transaction
        .execute(
            "UPDATE search_outbox_state SET indexed_sequence=MAX(indexed_sequence,MIN(latest_sequence,?1)) WHERE singleton=1",
            [through],
        )
        .storage()?;
    transaction
        .execute(
            "DELETE FROM search_outbox WHERE sequence<=(SELECT indexed_sequence FROM search_outbox_state WHERE singleton=1)",
            [],
        )
        .storage()?;
    transaction.commit().storage()?;
    Ok(())
}

#[async_trait]
impl SearchSynchronizationStorage for SqliteStorage {
    /// Read the durable latest and acknowledged revision watermarks.
    async fn search_status(&self) -> Result<SearchSynchronization, StorageError> {
        self.run(|connection| read_search_status(connection)).await
    }

    /// Read synchronization state and pending operations from one SQL snapshot.
    async fn search_batch(
        &self,
        after: u64,
        limit: usize,
    ) -> Result<(SearchSynchronization, Vec<SearchOperation>), StorageError> {
        self.run(move |connection| search_batch_blocking(connection, after, limit))
            .await
    }

    /// Read a bounded ordered batch after an exclusive synchronization cursor.
    async fn pending_search_operations(
        &self,
        after: u64,
        limit: usize,
    ) -> Result<Vec<SearchOperation>, StorageError> {
        self.run(move |connection| read_search_operations(connection, after, limit))
            .await
    }

    /// Advance the acknowledged watermark and prune its inclusive prefix atomically.
    async fn acknowledge_search_operations(&self, through: u64) -> Result<(), StorageError> {
        self.run(move |connection| acknowledge_search_operations_blocking(connection, through))
            .await
    }
}
