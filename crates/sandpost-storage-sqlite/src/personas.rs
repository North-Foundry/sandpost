//! Persona definitions and durable execution history on SQLite.
//!
//! Executions are scheduled by message insertion (see `messages::writes`); this module manages
//! Personas, the execution lifecycle, recorded steps, retention, and authorized history reads.
use crate::SqliteStorage;
use crate::error::StorageResult;
use crate::filter::{SqlValue, bind_query, compile};
use crate::records::{invalid_column, parse_identifier};
use async_trait::async_trait;
use sandpost_core::{
    Persona, PersonaActionDetail, PersonaActionStatus, PersonaExecution, PersonaExecutionAction,
    PersonaExecutionIdentifier, PersonaExecutionStatus, PersonaIdentifier, UserIdentifier,
};
use sandpost_storage::{
    FilterExpression, NewPersona, PERSONA_EXECUTION_RETENTION, PersonaExecutionEntry,
    PersonaStorage, PersonaUpdate, StorageError,
};
use sqlx::{Row, SqliteConnection, sqlite::SqliteRow};

const PERSONA_COLUMNS: &str =
    "identifier,owner_identifier,name,address,icon,script,enabled,revision,created_at,updated_at";

const EXECUTION_COLUMNS: &str = "execution.sequence,execution.identifier,execution.persona_identifier,execution.message_identifier,execution.status,execution.scheduled_at,execution.started_at,execution.completed_at,execution.persona_revision,execution.error_message";

const ACTION_COLUMNS: &str =
    "step,status,source_line,source_column,started_at,duration_milliseconds,error_message,detail";

/// Largest history page returned by one read.
const MAXIMUM_EXECUTION_PAGE: usize = 100;

/// SQL list of the statuses an execution can no longer leave.
const TERMINAL_STATUSES: &str = "('completed','failed','cancelled')";

/**
 * Convert a stored non-negative integer column into an unsigned value.
 */
fn unsigned(value: i64, column: &str) -> Result<u64, sqlx::Error> {
    u64::try_from(value).map_err(|_| invalid_column(column))
}

/**
 * Decode one Persona row.
 */
fn decode_persona(row: SqliteRow) -> Result<Persona, sqlx::Error> {
    let identifier: String = row.try_get("identifier")?;
    let owner: String = row.try_get("owner_identifier")?;
    let enabled: i64 = row.try_get("enabled")?;
    Ok(Persona {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column("identifier"))?,
        owner_identifier: parse_identifier(&owner)
            .map_err(|_| invalid_column("owner_identifier"))?,
        name: row.try_get("name")?,
        address: row.try_get("address")?,
        icon: row.try_get("icon")?,
        script: row.try_get("script")?,
        enabled: enabled != 0,
        revision: unsigned(row.try_get("revision")?, "revision")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/**
 * Decode one execution row selected with [`EXECUTION_COLUMNS`].
 */
fn decode_execution(row: SqliteRow) -> Result<PersonaExecution, sqlx::Error> {
    let identifier: String = row.try_get("identifier")?;
    let persona: String = row.try_get("persona_identifier")?;
    let message: String = row.try_get("message_identifier")?;
    let status: String = row.try_get("status")?;
    let revision: Option<i64> = row.try_get("persona_revision")?;
    Ok(PersonaExecution {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column("identifier"))?,
        sequence: unsigned(row.try_get("sequence")?, "sequence")?,
        persona_identifier: parse_identifier(&persona)
            .map_err(|_| invalid_column("persona_identifier"))?,
        message_identifier: parse_identifier(&message)
            .map_err(|_| invalid_column("message_identifier"))?,
        status: PersonaExecutionStatus::parse(&status).ok_or_else(|| invalid_column("status"))?,
        scheduled_at: row.try_get("scheduled_at")?,
        started_at: row.try_get("started_at")?,
        completed_at: row.try_get("completed_at")?,
        persona_revision: revision
            .map(|revision| unsigned(revision, "persona_revision"))
            .transpose()?,
        error_message: row.try_get("error_message")?,
    })
}

/**
 * Decode one recorded step row selected with [`ACTION_COLUMNS`].
 */
fn decode_action(row: SqliteRow) -> Result<PersonaExecutionAction, sqlx::Error> {
    let status: String = row.try_get("status")?;
    let detail: String = row.try_get("detail")?;
    let duration: Option<i64> = row.try_get("duration_milliseconds")?;
    let small = |value: i64, column: &str| u32::try_from(value).map_err(|_| invalid_column(column));
    Ok(PersonaExecutionAction {
        step: small(row.try_get("step")?, "step")?,
        status: PersonaActionStatus::parse(&status).ok_or_else(|| invalid_column("status"))?,
        line: small(row.try_get("source_line")?, "source_line")?,
        column: small(row.try_get("source_column")?, "source_column")?,
        started_at: row.try_get("started_at")?,
        duration_milliseconds: duration
            .map(|duration| unsigned(duration, "duration_milliseconds"))
            .transpose()?,
        error_message: row.try_get("error_message")?,
        detail: serde_json::from_str::<PersonaActionDetail>(&detail)
            .map_err(|_| invalid_column("detail"))?,
    })
}

/**
 * Map a uniqueness failure on the Persona address to its semantic error.
 */
fn persona_write_error(error: sqlx::Error) -> StorageError {
    if let sqlx::Error::Database(database) = &error
        && database.message().contains("personas.address")
    {
        return StorageError::DuplicatePersonaAddress;
    }
    crate::error::sqlite_error(error)
}

/**
 * Read one Persona inside an existing connection or transaction.
 */
async fn read_persona(
    connection: &mut SqliteConnection,
    identifier: PersonaIdentifier,
) -> Result<Option<Persona>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {PERSONA_COLUMNS} FROM personas WHERE identifier=?1"
    )))
    .bind(identifier.to_string())
    .fetch_optional(connection)
    .await
    .storage()?
    .map(decode_persona)
    .transpose()
    .storage()
}

/**
 * Read one execution inside an existing connection or transaction.
 */
async fn read_execution(
    connection: &mut SqliteConnection,
    identifier: PersonaExecutionIdentifier,
) -> Result<Option<PersonaExecution>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {EXECUTION_COLUMNS} FROM persona_executions AS execution WHERE execution.identifier=?1"
    )))
    .bind(identifier.to_string())
    .fetch_optional(connection)
    .await
    .storage()?
    .map(decode_execution)
    .transpose()
    .storage()
}

/**
 * Report whether another Persona already uses a normalized address.
 */
async fn address_taken(
    connection: &mut SqliteConnection,
    address: &str,
    excluded: Option<PersonaIdentifier>,
) -> Result<bool, StorageError> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM personas WHERE address=?1 AND identifier IS NOT ?2)",
    )
    .bind(address)
    .bind(excluded.map(|identifier| identifier.to_string()))
    .fetch_one(connection)
    .await
    .storage()
}

/**
 * Create a Persona after checking its owner and address inside one immediate transaction.
 */
async fn create_persona(
    storage: &SqliteStorage,
    persona: &NewPersona,
) -> Result<Persona, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let owner_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE identifier=?1)")
            .bind(persona.owner_identifier.to_string())
            .fetch_one(&mut *transaction)
            .await
            .storage()?;
    if !owner_exists {
        return Err(StorageError::NotFound);
    }
    if address_taken(&mut transaction, &persona.address, None).await? {
        return Err(StorageError::DuplicatePersonaAddress);
    }
    sqlx::query(
        "INSERT INTO personas(identifier,owner_identifier,name,address,icon,script,enabled,revision,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,1,?8,?8)",
    )
    .bind(persona.identifier.to_string())
    .bind(persona.owner_identifier.to_string())
    .bind(&persona.name)
    .bind(&persona.address)
    .bind(&persona.icon)
    .bind(&persona.script)
    .bind(i64::from(persona.enabled))
    .bind(persona.created_at)
    .execute(&mut *transaction)
    .await
    .map_err(persona_write_error)?;
    let created = read_persona(&mut transaction, persona.identifier)
        .await?
        .ok_or(StorageError::NotFound)?;
    transaction.commit().await.storage()?;
    Ok(created)
}

/**
 * Replace a Persona's mutable fields, preserving its execution revision for icon-only edits.
 */
async fn update_persona(
    storage: &SqliteStorage,
    identifier: PersonaIdentifier,
    update: &PersonaUpdate,
    updated_at: i64,
) -> Result<Option<Persona>, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let Some(previous) = read_persona(&mut transaction, identifier).await? else {
        return Ok(None);
    };
    if address_taken(&mut transaction, &update.address, Some(identifier)).await? {
        return Err(StorageError::DuplicatePersonaAddress);
    }
    let icon_only_change = previous.icon != update.icon
        && previous.name == update.name
        && previous.address == update.address
        && previous.script == update.script
        && previous.enabled == update.enabled;
    sqlx::query(
        "UPDATE personas SET name=?2,address=?3,icon=?4,script=?5,enabled=?6,revision=revision+?8,updated_at=?7 WHERE identifier=?1",
    )
    .bind(identifier.to_string())
    .bind(&update.name)
    .bind(&update.address)
    .bind(&update.icon)
    .bind(&update.script)
    .bind(i64::from(update.enabled))
    .bind(updated_at)
    .bind(i64::from(!icon_only_change))
    .execute(&mut *transaction)
    .await
    .map_err(persona_write_error)?;
    let updated = read_persona(&mut transaction, identifier).await?;
    transaction.commit().await.storage()?;
    Ok(updated)
}

/**
 * Move a running or pending execution to a terminal status and prune old history.
 */
async fn finish_persona_execution(
    storage: &SqliteStorage,
    identifier: PersonaExecutionIdentifier,
    status: PersonaExecutionStatus,
    completed_at: i64,
    error_message: Option<&str>,
) -> Result<bool, StorageError> {
    if !status.is_terminal() {
        return Err(StorageError::InvalidData(format!(
            "{} is not a terminal execution status",
            status.as_str()
        )));
    }
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let Some(execution) = read_execution(&mut transaction, identifier).await? else {
        return Ok(false);
    };
    if execution.status.is_terminal() {
        return Ok(false);
    }
    let unfinished_step = if status == PersonaExecutionStatus::Cancelled {
        PersonaActionStatus::Cancelled
    } else {
        PersonaActionStatus::Failed
    };
    sqlx::query(
        "UPDATE persona_execution_actions SET status=?2 WHERE execution_identifier=?1 AND status='started'",
    )
    .bind(identifier.to_string())
    .bind(unfinished_step.as_str())
    .execute(&mut *transaction)
    .await
    .storage()?;
    sqlx::query(
        "UPDATE persona_executions SET status=?2,completed_at=?3,error_message=?4 WHERE identifier=?1",
    )
    .bind(identifier.to_string())
    .bind(status.as_str())
    .bind(completed_at)
    .bind(error_message)
    .execute(&mut *transaction)
    .await
    .storage()?;
    let retention =
        i64::try_from(PERSONA_EXECUTION_RETENTION).map_err(|_| StorageError::IntegerRange)?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DELETE FROM persona_executions WHERE persona_identifier=?1 AND status IN {TERMINAL_STATUSES} AND sequence NOT IN (SELECT sequence FROM persona_executions WHERE persona_identifier=?1 AND status IN {TERMINAL_STATUSES} ORDER BY sequence DESC LIMIT ?2)"
    )))
    .bind(execution.persona_identifier.to_string())
    .bind(retention)
    .execute(&mut *transaction)
    .await
    .storage()?;
    transaction.commit().await.storage()?;
    Ok(true)
}

/**
 * List a Persona's executions whose messages the viewer can read, newest first.
 */
async fn list_persona_executions(
    connection: &mut SqliteConnection,
    persona: PersonaIdentifier,
    viewer: UserIdentifier,
    filter: &FilterExpression,
    before: Option<u64>,
    limit: usize,
) -> Result<Vec<PersonaExecutionEntry>, StorageError> {
    let compiled = compile(filter);
    let mut parameters = vec![SqlValue::Text(persona.to_string())];
    parameters.extend(compiled.parameters);
    parameters.push(SqlValue::Text(viewer.to_string()));
    let before_clause = match before {
        Some(sequence) => {
            parameters.push(SqlValue::Integer(
                i64::try_from(sequence).map_err(|_| StorageError::IntegerRange)?,
            ));
            "execution.sequence < ?"
        }
        None => "1",
    };
    parameters.push(SqlValue::Integer(
        i64::try_from(limit.min(MAXIMUM_EXECUTION_PAGE)).map_err(|_| StorageError::IntegerRange)?,
    ));
    let sql = format!(
        "SELECT {EXECUTION_COLUMNS},mail.subject AS message_subject FROM persona_executions AS execution JOIN mail ON mail.identifier = execution.message_identifier WHERE execution.persona_identifier = ? AND ({}) AND NOT EXISTS (SELECT 1 FROM user_message_deletions WHERE user_identifier = ? AND mail_sequence = mail.sequence) AND ({before_clause}) ORDER BY execution.sequence DESC LIMIT ?",
        compiled.sql
    );
    bind_query(sqlx::query(sqlx::AssertSqlSafe(sql)), &parameters)
        .fetch_all(connection)
        .await
        .storage()?
        .into_iter()
        .map(|row| {
            let subject: String = row.try_get("message_subject")?;
            Ok(PersonaExecutionEntry {
                execution: decode_execution(row)?,
                subject,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .storage()
}

#[async_trait]
impl PersonaStorage for SqliteStorage {
    /**
     * Create a Persona with database-enforced global address uniqueness.
     */
    async fn create_persona(&self, persona: &NewPersona) -> Result<Persona, StorageError> {
        create_persona(self, persona).await
    }

    /**
     * Load one Persona.
     */
    async fn get_persona(
        &self,
        identifier: PersonaIdentifier,
    ) -> Result<Option<Persona>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        read_persona(&mut connection, identifier).await
    }

    /**
     * List one owner's Personas, or all of them, by name and identifier.
     */
    async fn list_personas(
        &self,
        owner: Option<UserIdentifier>,
    ) -> Result<Vec<Persona>, StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {PERSONA_COLUMNS} FROM personas WHERE ?1 IS NULL OR owner_identifier=?1 ORDER BY name,identifier"
        )))
        .bind(owner.map(|owner| owner.to_string()))
        .fetch_all(&self.read_pool)
        .await
        .storage()?
        .into_iter()
        .map(decode_persona)
        .collect::<Result<Vec<_>, _>>()
        .storage()
    }

    /**
     * Replace a Persona's mutable fields and advance its revision.
     */
    async fn update_persona(
        &self,
        identifier: PersonaIdentifier,
        update: &PersonaUpdate,
        updated_at: i64,
    ) -> Result<Option<Persona>, StorageError> {
        update_persona(self, identifier, update, updated_at).await
    }

    /**
     * Delete a Persona; its executions and their steps cascade.
     */
    async fn delete_persona(&self, identifier: PersonaIdentifier) -> Result<bool, StorageError> {
        let result = sqlx::query("DELETE FROM personas WHERE identifier=?1")
            .bind(identifier.to_string())
            .execute(&self.write_pool)
            .await
            .storage()?;
        Ok(result.rows_affected() > 0)
    }

    /**
     * List executions in one status in scheduling order after a sequence cursor.
     */
    async fn list_persona_executions_in_status(
        &self,
        status: PersonaExecutionStatus,
        after: Option<u64>,
        limit: usize,
    ) -> Result<Vec<PersonaExecution>, StorageError> {
        let after = after
            .map(i64::try_from)
            .transpose()
            .map_err(|_| StorageError::IntegerRange)?
            .unwrap_or(0);
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {EXECUTION_COLUMNS} FROM persona_executions AS execution WHERE execution.status=?1 AND execution.sequence>?2 ORDER BY execution.sequence LIMIT ?3"
        )))
        .bind(status.as_str())
        .bind(after)
        .bind(i64::try_from(limit).map_err(|_| StorageError::IntegerRange)?)
        .fetch_all(&self.read_pool)
        .await
        .storage()?
        .into_iter()
        .map(decode_execution)
        .collect::<Result<Vec<_>, _>>()
        .storage()
    }

    /**
     * Start a pending execution, or return a running one unchanged for resumption.
     */
    async fn start_persona_execution(
        &self,
        identifier: PersonaExecutionIdentifier,
        started_at: i64,
        persona_revision: u64,
    ) -> Result<Option<PersonaExecution>, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        let Some(execution) = read_execution(&mut transaction, identifier).await? else {
            return Ok(None);
        };
        let started = match execution.status {
            PersonaExecutionStatus::Running => Some(execution),
            PersonaExecutionStatus::Pending => {
                sqlx::query(
                    "UPDATE persona_executions SET status='running',started_at=?2,persona_revision=?3 WHERE identifier=?1",
                )
                .bind(identifier.to_string())
                .bind(started_at)
                .bind(i64::try_from(persona_revision).map_err(|_| StorageError::IntegerRange)?)
                .execute(&mut *transaction)
                .await
                .storage()?;
                read_execution(&mut transaction, identifier).await?
            }
            _ => None,
        };
        transaction.commit().await.storage()?;
        Ok(started)
    }

    /**
     * Insert or replace one recorded step of an existing execution.
     */
    async fn record_persona_execution_action(
        &self,
        identifier: PersonaExecutionIdentifier,
        action: &PersonaExecutionAction,
    ) -> Result<(), StorageError> {
        let detail = serde_json::to_string(&action.detail)
            .map_err(|error| StorageError::InvalidData(error.to_string()))?;
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        if read_execution(&mut transaction, identifier)
            .await?
            .is_none()
        {
            return Err(StorageError::NotFound);
        }
        sqlx::query(
            "INSERT INTO persona_execution_actions(execution_identifier,step,kind,status,source_line,source_column,started_at,duration_milliseconds,error_message,detail) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(execution_identifier,step) DO UPDATE SET kind=excluded.kind,status=excluded.status,source_line=excluded.source_line,source_column=excluded.source_column,started_at=excluded.started_at,duration_milliseconds=excluded.duration_milliseconds,error_message=excluded.error_message,detail=excluded.detail",
        )
        .bind(identifier.to_string())
        .bind(i64::from(action.step))
        .bind(action.detail.kind().as_str())
        .bind(action.status.as_str())
        .bind(i64::from(action.line.max(1)))
        .bind(i64::from(action.column.max(1)))
        .bind(action.started_at)
        .bind(
            action
                .duration_milliseconds
                .map(i64::try_from)
                .transpose()
                .map_err(|_| StorageError::IntegerRange)?,
        )
        .bind(&action.error_message)
        .bind(detail)
        .execute(&mut *transaction)
        .await
        .storage()?;
        transaction.commit().await.storage()?;
        Ok(())
    }

    /**
     * Finish a pending or running execution and apply history retention.
     */
    async fn finish_persona_execution(
        &self,
        identifier: PersonaExecutionIdentifier,
        status: PersonaExecutionStatus,
        completed_at: i64,
        error_message: Option<&str>,
    ) -> Result<bool, StorageError> {
        finish_persona_execution(self, identifier, status, completed_at, error_message).await
    }

    /**
     * Cancel every unfinished execution of a Persona.
     */
    async fn cancel_persona_executions(
        &self,
        persona: PersonaIdentifier,
        reason: &str,
        completed_at: i64,
    ) -> Result<u64, StorageError> {
        let mut transaction = self
            .write_pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .storage()?;
        sqlx::query(
            "UPDATE persona_execution_actions SET status='cancelled' WHERE status='started' AND execution_identifier IN (SELECT identifier FROM persona_executions WHERE persona_identifier=?1 AND status IN ('pending','running'))",
        )
        .bind(persona.to_string())
        .execute(&mut *transaction)
        .await
        .storage()?;
        let result = sqlx::query(
            "UPDATE persona_executions SET status='cancelled',completed_at=?2,error_message=?3 WHERE persona_identifier=?1 AND status IN ('pending','running')",
        )
        .bind(persona.to_string())
        .bind(completed_at)
        .bind(reason)
        .execute(&mut *transaction)
        .await
        .storage()?;
        transaction.commit().await.storage()?;
        Ok(result.rows_affected())
    }

    /**
     * Remove one execution and its steps.
     */
    async fn discard_persona_execution(
        &self,
        identifier: PersonaExecutionIdentifier,
    ) -> Result<bool, StorageError> {
        let result = sqlx::query("DELETE FROM persona_executions WHERE identifier=?1")
            .bind(identifier.to_string())
            .execute(&self.write_pool)
            .await
            .storage()?;
        Ok(result.rows_affected() > 0)
    }

    /**
     * List a Persona's executions limited to messages the viewer can read.
     */
    async fn list_persona_executions(
        &self,
        persona: PersonaIdentifier,
        viewer: UserIdentifier,
        filter: &FilterExpression,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<PersonaExecutionEntry>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let executions =
            list_persona_executions(&mut transaction, persona, viewer, filter, before, limit)
                .await?;
        transaction.commit().await.storage()?;
        Ok(executions)
    }

    /**
     * Load an execution and its steps from one snapshot.
     */
    async fn get_persona_execution(
        &self,
        identifier: PersonaExecutionIdentifier,
    ) -> Result<Option<(PersonaExecution, Vec<PersonaExecutionAction>)>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let Some(execution) = read_execution(&mut transaction, identifier).await? else {
            return Ok(None);
        };
        let actions = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {ACTION_COLUMNS} FROM persona_execution_actions WHERE execution_identifier=?1 ORDER BY step"
        )))
        .bind(identifier.to_string())
        .fetch_all(&mut *transaction)
        .await
        .storage()?
        .into_iter()
        .map(decode_action)
        .collect::<Result<Vec<_>, _>>()
        .storage()?;
        transaction.commit().await.storage()?;
        Ok(Some((execution, actions)))
    }
}
