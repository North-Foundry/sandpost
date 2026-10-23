//! SMTP Outbounds, scope routing actions and durable outbound delivery jobs on SQLite.
//!
//! Routing evaluations are queued by message insertion (see `messages::writes`); this module
//! manages the configuration, turns evaluations into jobs, and records every attempt.
use crate::SqliteStorage;
use crate::error::StorageResult;
use crate::filter::{SqlValue, bind_query, compile};
use crate::records::{invalid_column, parse_identifier};
use async_trait::async_trait;
use sandpost_core::{
    MessageIdentifier, OutboundAttemptOutcome, OutboundDelivery, OutboundDeliveryAttempt,
    OutboundDeliveryIdentifier, OutboundDeliveryStatus, OutboundFailure, OutboundFailureKind,
    OutboundRecipient, OutboundRecipientStatus, RoutingAction, RoutingActionIdentifier,
    RoutingActionKind, ScopeIdentifier, SmtpOutbound, SmtpOutboundAuthentication,
    SmtpOutboundIdentifier, SmtpOutboundSecurity,
};
use sandpost_storage::{
    NewOutboundDelivery, NewRoutingAction, NewSmtpOutbound, OUTBOUND_ATTEMPT_RETENTION,
    OUTBOUND_DELIVERY_RETENTION, OutboundAttemptResult, OutboundDeliveryQuery, OutboundStorage,
    RoutingActionUpdate, SmtpOutboundCredential, SmtpOutboundPassword, SmtpOutboundSettings,
    StorageError,
};
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection, sqlite::SqliteRow};
use std::collections::HashMap;

const OUTBOUND_COLUMNS: &str = "identifier,name,host,port,security,authentication,username,encrypted_password IS NOT NULL AS password_configured,enabled,override_from,envelope_from,connect_timeout_seconds,smtp_timeout_seconds,verify_certificates,helo_name,maximum_connections,allowed_recipients,created_at,updated_at";

const ACTION_COLUMNS: &str = "identifier,scope_identifier,smtp_outbound_identifier,kind,forward_recipients,enabled,created_at,updated_at";

const DELIVERY_COLUMNS: &str = "delivery.sequence,delivery.identifier,delivery.message_identifier,mail.subject,delivery.scope_identifier,delivery.routing_action_identifier,delivery.smtp_outbound_identifier,delivery.smtp_outbound_name,delivery.kind,delivery.status,delivery.envelope_from,delivery.attempt_count,delivery.error_kind,delivery.error_message,delivery.error_code,delivery.created_at,delivery.last_attempt_at,delivery.next_attempt_at,delivery.delivered_at";

const RECIPIENT_COLUMNS: &str =
    "delivery_identifier,address,status,error_kind,error_message,error_code,response,delivered_at";

const ATTEMPT_COLUMNS: &str = "number,started_at,finished_at,outcome,error_kind,error_message,error_code,delivered_recipients,deferred_recipients,failed_recipients";

/// Largest delivery page returned by one read.
const MAXIMUM_DELIVERY_PAGE: usize = 100;

/// Statuses a delivery waits in before an attempt claims it.
const WAITING_STATUSES: &str = "('pending','retrying')";

/// Why a restart failed an attempt in progress.
const INTERRUPTED_ATTEMPT: &str = "the service stopped during this attempt; the server may already have accepted the message, so the next attempt can deliver a duplicate";

/**
 * Convert a stored non-negative integer column into a 32-bit unsigned value.
 */
fn small(value: i64, column: &str) -> Result<u32, sqlx::Error> {
    u32::try_from(value).map_err(|_| invalid_column(column))
}

/**
 * Decode a stored JSON array of strings.
 */
fn string_list(value: &str, column: &str) -> Result<Vec<String>, sqlx::Error> {
    serde_json::from_str(value).map_err(|_| invalid_column(column))
}

/**
 * Encode a list of strings as the JSON array stored in a column.
 */
fn encode_list(values: &[String]) -> Result<String, StorageError> {
    serde_json::to_string(values).map_err(|error| StorageError::InvalidData(error.to_string()))
}

/**
 * Decode the optional failure stored in the `error_kind`, `error_message` and `error_code`
 * columns of a row.
 */
fn decode_failure(row: &SqliteRow) -> Result<Option<OutboundFailure>, sqlx::Error> {
    let kind: Option<String> = row.try_get("error_kind")?;
    let Some(kind) = kind else {
        return Ok(None);
    };
    let code: Option<i64> = row.try_get("error_code")?;
    Ok(Some(OutboundFailure {
        kind: OutboundFailureKind::parse(&kind).ok_or_else(|| invalid_column("error_kind"))?,
        message: row
            .try_get::<Option<String>, _>("error_message")?
            .unwrap_or_default(),
        code: code
            .map(|code| u16::try_from(code).map_err(|_| invalid_column("error_code")))
            .transpose()?,
    }))
}

/**
 * Split an optional failure into the values of its three stored columns.
 */
fn failure_columns(failure: Option<&OutboundFailure>) -> (Option<&str>, Option<&str>, Option<i64>) {
    match failure {
        Some(failure) => (
            Some(failure.kind.as_str()),
            Some(failure.message.as_str()),
            failure.code.map(i64::from),
        ),
        None => (None, None, None),
    }
}

/**
 * Decode one SMTP Outbound row selected with [`OUTBOUND_COLUMNS`].
 */
fn decode_outbound(row: &SqliteRow) -> Result<SmtpOutbound, sqlx::Error> {
    let identifier: String = row.try_get("identifier")?;
    let security: String = row.try_get("security")?;
    let authentication: String = row.try_get("authentication")?;
    let recipients: String = row.try_get("allowed_recipients")?;
    let optional_small = |column: &str| -> Result<Option<u32>, sqlx::Error> {
        row.try_get::<Option<i64>, _>(column)?
            .map(|value| small(value, column))
            .transpose()
    };
    Ok(SmtpOutbound {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column("identifier"))?,
        name: row.try_get("name")?,
        host: row.try_get("host")?,
        port: u16::try_from(row.try_get::<i64, _>("port")?).map_err(|_| invalid_column("port"))?,
        security: SmtpOutboundSecurity::parse(&security)
            .ok_or_else(|| invalid_column("security"))?,
        authentication: SmtpOutboundAuthentication::parse(&authentication)
            .ok_or_else(|| invalid_column("authentication"))?,
        username: row.try_get("username")?,
        password_configured: row.try_get::<i64, _>("password_configured")? != 0,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        override_from: row.try_get("override_from")?,
        envelope_from: row.try_get("envelope_from")?,
        connect_timeout_seconds: optional_small("connect_timeout_seconds")?,
        smtp_timeout_seconds: optional_small("smtp_timeout_seconds")?,
        verify_certificates: row.try_get::<i64, _>("verify_certificates")? != 0,
        helo_name: row.try_get("helo_name")?,
        maximum_connections: optional_small("maximum_connections")?,
        allowed_recipients: string_list(&recipients, "allowed_recipients")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/**
 * Decode one routing action row selected with [`ACTION_COLUMNS`].
 */
fn decode_action(row: &SqliteRow) -> Result<RoutingAction, sqlx::Error> {
    let identifier: String = row.try_get("identifier")?;
    let scope: String = row.try_get("scope_identifier")?;
    let outbound: String = row.try_get("smtp_outbound_identifier")?;
    let kind: String = row.try_get("kind")?;
    let recipients: String = row.try_get("forward_recipients")?;
    Ok(RoutingAction {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column("identifier"))?,
        scope_identifier: parse_identifier(&scope)
            .map_err(|_| invalid_column("scope_identifier"))?,
        smtp_outbound_identifier: parse_identifier(&outbound)
            .map_err(|_| invalid_column("smtp_outbound_identifier"))?,
        kind: RoutingActionKind::parse(&kind).ok_or_else(|| invalid_column("kind"))?,
        forward_recipients: string_list(&recipients, "forward_recipients")?,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/**
 * Decode an optional identifier column.
 */
fn optional_identifier<IdentifierType: std::str::FromStr>(
    row: &SqliteRow,
    column: &str,
) -> Result<Option<IdentifierType>, sqlx::Error> {
    row.try_get::<Option<String>, _>(column)?
        .map(|value| parse_identifier(&value).map_err(|_| invalid_column(column)))
        .transpose()
}

/**
 * Decode one delivery row selected with [`DELIVERY_COLUMNS`], without its recipients.
 */
fn decode_delivery(row: &SqliteRow) -> Result<OutboundDelivery, sqlx::Error> {
    let identifier: String = row.try_get("identifier")?;
    let message: String = row.try_get("message_identifier")?;
    let kind: String = row.try_get("kind")?;
    let status: String = row.try_get("status")?;
    Ok(OutboundDelivery {
        identifier: parse_identifier(&identifier).map_err(|_| invalid_column("identifier"))?,
        sequence: u64::try_from(row.try_get::<i64, _>("sequence")?)
            .map_err(|_| invalid_column("sequence"))?,
        message_identifier: parse_identifier(&message)
            .map_err(|_| invalid_column("message_identifier"))?,
        subject: row.try_get("subject")?,
        scope_identifier: optional_identifier(row, "scope_identifier")?,
        routing_action_identifier: optional_identifier(row, "routing_action_identifier")?,
        smtp_outbound_identifier: optional_identifier(row, "smtp_outbound_identifier")?,
        smtp_outbound_name: row.try_get("smtp_outbound_name")?,
        kind: RoutingActionKind::parse(&kind).ok_or_else(|| invalid_column("kind"))?,
        status: OutboundDeliveryStatus::parse(&status).ok_or_else(|| invalid_column("status"))?,
        envelope_from: row.try_get("envelope_from")?,
        attempt_count: small(row.try_get("attempt_count")?, "attempt_count")?,
        last_error: decode_failure(row)?,
        created_at: row.try_get("created_at")?,
        last_attempt_at: row.try_get("last_attempt_at")?,
        next_attempt_at: row.try_get("next_attempt_at")?,
        delivered_at: row.try_get("delivered_at")?,
        recipients: Vec::new(),
    })
}

/**
 * Decode one recipient row selected with [`RECIPIENT_COLUMNS`], returning its delivery.
 */
fn decode_recipient(row: &SqliteRow) -> Result<(String, OutboundRecipient), sqlx::Error> {
    let status: String = row.try_get("status")?;
    Ok((
        row.try_get("delivery_identifier")?,
        OutboundRecipient {
            address: row.try_get("address")?,
            status: OutboundRecipientStatus::parse(&status)
                .ok_or_else(|| invalid_column("status"))?,
            error: decode_failure(row)?,
            response: row.try_get("response")?,
            delivered_at: row.try_get("delivered_at")?,
        },
    ))
}

/**
 * Decode one attempt row selected with [`ATTEMPT_COLUMNS`].
 */
fn decode_attempt(row: &SqliteRow) -> Result<OutboundDeliveryAttempt, sqlx::Error> {
    let outcome: Option<String> = row.try_get("outcome")?;
    Ok(OutboundDeliveryAttempt {
        number: small(row.try_get("number")?, "number")?,
        started_at: row.try_get("started_at")?,
        finished_at: row.try_get("finished_at")?,
        outcome: outcome
            .map(|outcome| {
                OutboundAttemptOutcome::parse(&outcome).ok_or_else(|| invalid_column("outcome"))
            })
            .transpose()?,
        error: decode_failure(row)?,
        delivered_recipients: small(row.try_get("delivered_recipients")?, "delivered_recipients")?,
        deferred_recipients: small(row.try_get("deferred_recipients")?, "deferred_recipients")?,
        failed_recipients: small(row.try_get("failed_recipients")?, "failed_recipients")?,
    })
}

/**
 * Map a uniqueness failure on the SMTP Outbound name to its semantic error.
 */
fn outbound_write_error(error: sqlx::Error) -> StorageError {
    if let sqlx::Error::Database(database) = &error
        && database.message().contains("smtp_outbounds.name")
    {
        return StorageError::DuplicateSmtpOutboundName;
    }
    crate::error::sqlite_error(error)
}

/**
 * Convert an optional unsigned setting into its stored integer.
 */
fn optional_integer(value: Option<u32>) -> Option<i64> {
    value.map(i64::from)
}

/**
 * Read one SMTP Outbound inside an existing connection or transaction.
 */
async fn read_outbound(
    connection: &mut SqliteConnection,
    identifier: SmtpOutboundIdentifier,
) -> Result<Option<SmtpOutbound>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {OUTBOUND_COLUMNS} FROM smtp_outbounds WHERE identifier=?1"
    )))
    .bind(identifier.to_string())
    .fetch_optional(connection)
    .await
    .storage()?
    .as_ref()
    .map(decode_outbound)
    .transpose()
    .storage()
}

/**
 * Read one routing action inside an existing connection or transaction.
 */
async fn read_action(
    connection: &mut SqliteConnection,
    identifier: RoutingActionIdentifier,
) -> Result<Option<RoutingAction>, StorageError> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {ACTION_COLUMNS} FROM scope_routing_actions WHERE identifier=?1"
    )))
    .bind(identifier.to_string())
    .fetch_optional(connection)
    .await
    .storage()?
    .as_ref()
    .map(decode_action)
    .transpose()
    .storage()
}

/**
 * Report whether another SMTP Outbound already uses a name, ignoring ASCII case.
 */
async fn name_taken(
    connection: &mut SqliteConnection,
    name: &str,
    excluded: Option<SmtpOutboundIdentifier>,
) -> Result<bool, StorageError> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM smtp_outbounds WHERE name=?1 COLLATE NOCASE AND identifier IS NOT ?2)",
    )
    .bind(name)
    .bind(excluded.map(|identifier| identifier.to_string()))
    .fetch_one(connection)
    .await
    .storage()
}

/**
 * Report whether a row with this identifier exists in `table`.
 */
async fn exists(
    connection: &mut SqliteConnection,
    table: &str,
    identifier: String,
) -> Result<bool, StorageError> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT EXISTS(SELECT 1 FROM {table} WHERE identifier=?1)"
    )))
    .bind(identifier)
    .fetch_one(connection)
    .await
    .storage()
}

/**
 * Fail the waiting deliveries whose `column` references `identifier`, together with their
 * pending recipients. Deliveries with an attempt in progress are left to finish it.
 */
async fn fail_waiting_deliveries(
    connection: &mut SqliteConnection,
    column: &str,
    identifier: String,
    failure: &OutboundFailure,
) -> Result<(), StorageError> {
    let (kind, message, code) = failure_columns(Some(failure));
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE outbound_delivery_recipients SET status='failed',error_kind=?2,error_message=?3,error_code=?4 WHERE status='pending' AND delivery_identifier IN (SELECT identifier FROM outbound_deliveries WHERE {column}=?1 AND status IN {WAITING_STATUSES})"
    )))
    .bind(&identifier)
    .bind(kind)
    .bind(message)
    .bind(code)
    .execute(&mut *connection)
    .await
    .storage()?;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE outbound_deliveries SET status='failed',error_kind=?2,error_message=?3,error_code=?4,next_attempt_at=NULL WHERE {column}=?1 AND status IN {WAITING_STATUSES}"
    )))
    .bind(&identifier)
    .bind(kind)
    .bind(message)
    .bind(code)
    .execute(&mut *connection)
    .await
    .storage()?;
    Ok(())
}

/**
 * Build a configuration failure explaining why queued deliveries stopped.
 */
fn configuration_failure(message: &str) -> OutboundFailure {
    OutboundFailure {
        kind: OutboundFailureKind::Configuration,
        message: message.to_owned(),
        code: None,
    }
}

/**
 * Attach every delivery's recipients, read in position order inside the same connection.
 */
async fn attach_recipients(
    connection: &mut SqliteConnection,
    deliveries: &mut [OutboundDelivery],
) -> Result<(), StorageError> {
    if deliveries.is_empty() {
        return Ok(());
    }
    let mut recipients: HashMap<String, Vec<OutboundRecipient>> = HashMap::new();
    for chunk in deliveries.chunks(128) {
        let mut statement = QueryBuilder::<Sqlite>::new(format!(
            "SELECT {RECIPIENT_COLUMNS} FROM outbound_delivery_recipients WHERE delivery_identifier IN ("
        ));
        let mut separated = statement.separated(", ");
        for delivery in chunk {
            separated.push_bind(delivery.identifier.to_string());
        }
        statement.push(") ORDER BY delivery_identifier, position");
        for row in statement
            .build()
            .fetch_all(&mut *connection)
            .await
            .storage()?
        {
            let (delivery, recipient) = decode_recipient(&row).storage()?;
            recipients.entry(delivery).or_default().push(recipient);
        }
    }
    for delivery in deliveries {
        delivery.recipients = recipients
            .remove(&delivery.identifier.to_string())
            .unwrap_or_default();
    }
    Ok(())
}

/**
 * Read one delivery with its recipients inside an existing connection or transaction.
 */
async fn read_delivery(
    connection: &mut SqliteConnection,
    identifier: OutboundDeliveryIdentifier,
) -> Result<Option<OutboundDelivery>, StorageError> {
    let Some(row) = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {DELIVERY_COLUMNS} FROM outbound_deliveries AS delivery JOIN mail ON mail.identifier=delivery.message_identifier WHERE delivery.identifier=?1"
    )))
    .bind(identifier.to_string())
    .fetch_optional(&mut *connection)
    .await
    .storage()?
    else {
        return Ok(None);
    };
    let mut deliveries = vec![decode_delivery(&row).storage()?];
    attach_recipients(connection, &mut deliveries).await?;
    Ok(deliveries.pop())
}

/**
 * Insert one SMTP Outbound inside an immediate transaction after checking its name.
 */
async fn create_outbound(
    storage: &SqliteStorage,
    outbound: &NewSmtpOutbound,
) -> Result<SmtpOutbound, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let settings = &outbound.settings;
    if name_taken(&mut transaction, &settings.name, None).await? {
        return Err(StorageError::DuplicateSmtpOutboundName);
    }
    sqlx::query(
        "INSERT INTO smtp_outbounds(identifier,name,host,port,security,authentication,username,encrypted_password,enabled,override_from,envelope_from,connect_timeout_seconds,smtp_timeout_seconds,verify_certificates,helo_name,maximum_connections,allowed_recipients,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?18)",
    )
    .bind(outbound.identifier.to_string())
    .bind(&settings.name)
    .bind(&settings.host)
    .bind(i64::from(settings.port))
    .bind(settings.security.as_str())
    .bind(settings.authentication.as_str())
    .bind(&settings.username)
    .bind(&outbound.encrypted_password)
    .bind(i64::from(settings.enabled))
    .bind(&settings.override_from)
    .bind(&settings.envelope_from)
    .bind(optional_integer(settings.connect_timeout_seconds))
    .bind(optional_integer(settings.smtp_timeout_seconds))
    .bind(i64::from(settings.verify_certificates))
    .bind(&settings.helo_name)
    .bind(optional_integer(settings.maximum_connections))
    .bind(encode_list(&settings.allowed_recipients)?)
    .bind(outbound.created_at)
    .execute(&mut *transaction)
    .await
    .map_err(outbound_write_error)?;
    let created = read_outbound(&mut transaction, outbound.identifier)
        .await?
        .ok_or(StorageError::NotFound)?;
    transaction.commit().await.storage()?;
    Ok(created)
}

/**
 * Replace an SMTP Outbound's settings and password inside an immediate transaction.
 */
async fn update_outbound(
    storage: &SqliteStorage,
    identifier: SmtpOutboundIdentifier,
    settings: &SmtpOutboundSettings,
    password: &SmtpOutboundPassword,
    updated_at: i64,
) -> Result<Option<SmtpOutbound>, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let Some(previous) = read_outbound(&mut transaction, identifier).await? else {
        return Ok(None);
    };
    if name_taken(&mut transaction, &settings.name, Some(identifier)).await? {
        return Err(StorageError::DuplicateSmtpOutboundName);
    }
    let (replace_password, encrypted_password) = match password {
        SmtpOutboundPassword::Keep => (0, None),
        SmtpOutboundPassword::Replace(encrypted) => (1, Some(encrypted.as_slice())),
        SmtpOutboundPassword::Clear => (1, None),
    };
    sqlx::query(
        "UPDATE smtp_outbounds SET name=?2,host=?3,port=?4,security=?5,authentication=?6,username=?7,encrypted_password=CASE WHEN ?8 THEN ?9 ELSE encrypted_password END,enabled=?10,override_from=?11,envelope_from=?12,connect_timeout_seconds=?13,smtp_timeout_seconds=?14,verify_certificates=?15,helo_name=?16,maximum_connections=?17,allowed_recipients=?18,updated_at=?19 WHERE identifier=?1",
    )
    .bind(identifier.to_string())
    .bind(&settings.name)
    .bind(&settings.host)
    .bind(i64::from(settings.port))
    .bind(settings.security.as_str())
    .bind(settings.authentication.as_str())
    .bind(&settings.username)
    .bind(replace_password)
    .bind(encrypted_password)
    .bind(i64::from(settings.enabled))
    .bind(&settings.override_from)
    .bind(&settings.envelope_from)
    .bind(optional_integer(settings.connect_timeout_seconds))
    .bind(optional_integer(settings.smtp_timeout_seconds))
    .bind(i64::from(settings.verify_certificates))
    .bind(&settings.helo_name)
    .bind(optional_integer(settings.maximum_connections))
    .bind(encode_list(&settings.allowed_recipients)?)
    .bind(updated_at)
    .execute(&mut *transaction)
    .await
    .map_err(outbound_write_error)?;
    if previous.enabled && !settings.enabled {
        fail_waiting_deliveries(
            &mut transaction,
            "smtp_outbound_identifier",
            identifier.to_string(),
            &configuration_failure("the SMTP Outbound was disabled"),
        )
        .await?;
    }
    let updated = read_outbound(&mut transaction, identifier).await?;
    transaction.commit().await.storage()?;
    Ok(updated)
}

/**
 * Delete an unreferenced SMTP Outbound and fail its waiting deliveries.
 */
async fn delete_outbound(
    storage: &SqliteStorage,
    identifier: SmtpOutboundIdentifier,
) -> Result<bool, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let references: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM scope_routing_actions WHERE smtp_outbound_identifier=?1",
    )
    .bind(identifier.to_string())
    .fetch_one(&mut *transaction)
    .await
    .storage()?;
    if references > 0 {
        return Err(StorageError::SmtpOutboundInUse(
            u64::try_from(references).map_err(|_| StorageError::IntegerRange)?,
        ));
    }
    fail_waiting_deliveries(
        &mut transaction,
        "smtp_outbound_identifier",
        identifier.to_string(),
        &configuration_failure("the SMTP Outbound was deleted"),
    )
    .await?;
    let deleted = sqlx::query("DELETE FROM smtp_outbounds WHERE identifier=?1")
        .bind(identifier.to_string())
        .execute(&mut *transaction)
        .await
        .storage()?
        .rows_affected()
        > 0;
    transaction.commit().await.storage()?;
    Ok(deleted)
}

/**
 * Insert a routing action after checking that its scope and SMTP Outbound exist.
 */
async fn create_action(
    storage: &SqliteStorage,
    action: &NewRoutingAction,
) -> Result<RoutingAction, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    if !exists(
        &mut transaction,
        "scopes",
        action.scope_identifier.to_string(),
    )
    .await?
        || !exists(
            &mut transaction,
            "smtp_outbounds",
            action.smtp_outbound_identifier.to_string(),
        )
        .await?
    {
        return Err(StorageError::NotFound);
    }
    sqlx::query(
        "INSERT INTO scope_routing_actions(identifier,scope_identifier,smtp_outbound_identifier,kind,forward_recipients,enabled,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?7)",
    )
    .bind(action.identifier.to_string())
    .bind(action.scope_identifier.to_string())
    .bind(action.smtp_outbound_identifier.to_string())
    .bind(action.kind.as_str())
    .bind(encode_list(&action.forward_recipients)?)
    .bind(i64::from(action.enabled))
    .bind(action.created_at)
    .execute(&mut *transaction)
    .await
    .storage()?;
    let created = read_action(&mut transaction, action.identifier)
        .await?
        .ok_or(StorageError::NotFound)?;
    transaction.commit().await.storage()?;
    Ok(created)
}

/**
 * Replace a routing action and fail its waiting deliveries when they no longer apply.
 */
async fn update_action(
    storage: &SqliteStorage,
    identifier: RoutingActionIdentifier,
    update: &RoutingActionUpdate,
    updated_at: i64,
) -> Result<Option<RoutingAction>, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let Some(previous) = read_action(&mut transaction, identifier).await? else {
        return Ok(None);
    };
    if !exists(
        &mut transaction,
        "smtp_outbounds",
        update.smtp_outbound_identifier.to_string(),
    )
    .await?
    {
        return Err(StorageError::NotFound);
    }
    sqlx::query(
        "UPDATE scope_routing_actions SET smtp_outbound_identifier=?2,kind=?3,forward_recipients=?4,enabled=?5,updated_at=?6 WHERE identifier=?1",
    )
    .bind(identifier.to_string())
    .bind(update.smtp_outbound_identifier.to_string())
    .bind(update.kind.as_str())
    .bind(encode_list(&update.forward_recipients)?)
    .bind(i64::from(update.enabled))
    .bind(updated_at)
    .execute(&mut *transaction)
    .await
    .storage()?;
    let reason = if previous.enabled && !update.enabled {
        Some("the routing action was disabled")
    } else if previous.kind != update.kind
        || previous.smtp_outbound_identifier != update.smtp_outbound_identifier
    {
        Some("the routing action changed before this delivery started")
    } else {
        None
    };
    if let Some(reason) = reason {
        fail_waiting_deliveries(
            &mut transaction,
            "routing_action_identifier",
            identifier.to_string(),
            &configuration_failure(reason),
        )
        .await?;
    }
    let updated = read_action(&mut transaction, identifier).await?;
    transaction.commit().await.storage()?;
    Ok(updated)
}

/**
 * Delete a routing action after failing its waiting deliveries.
 */
async fn delete_action(
    storage: &SqliteStorage,
    identifier: RoutingActionIdentifier,
) -> Result<bool, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    fail_waiting_deliveries(
        &mut transaction,
        "routing_action_identifier",
        identifier.to_string(),
        &configuration_failure("the routing action was deleted"),
    )
    .await?;
    let deleted = sqlx::query("DELETE FROM scope_routing_actions WHERE identifier=?1")
        .bind(identifier.to_string())
        .execute(&mut *transaction)
        .await
        .storage()?
        .rows_affected()
        > 0;
    transaction.commit().await.storage()?;
    Ok(deleted)
}

/**
 * Create the decided deliveries of a queued message and dequeue it in one transaction.
 */
async fn complete_evaluation(
    storage: &SqliteStorage,
    message: MessageIdentifier,
    deliveries: &[NewOutboundDelivery],
) -> Result<u64, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let dequeued = sqlx::query("DELETE FROM mail_routing_evaluations WHERE message_identifier=?1")
        .bind(message.to_string())
        .execute(&mut *transaction)
        .await
        .storage()?
        .rows_affected();
    if dequeued == 0 {
        transaction.commit().await.storage()?;
        return Ok(0);
    }
    let mut created = 0;
    for delivery in deliveries {
        let (kind, error_message, code) = failure_columns(delivery.last_error.as_ref());
        let waiting = delivery.status == OutboundDeliveryStatus::Pending;
        let inserted = sqlx::query(
            "INSERT INTO outbound_deliveries(identifier,message_identifier,scope_identifier,routing_action_identifier,smtp_outbound_identifier,smtp_outbound_name,kind,status,error_kind,error_message,error_code,created_at,next_attempt_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13) ON CONFLICT(message_identifier,routing_action_identifier) DO NOTHING",
        )
        .bind(delivery.identifier.to_string())
        .bind(message.to_string())
        .bind(delivery.scope_identifier.to_string())
        .bind(delivery.routing_action_identifier.to_string())
        .bind(delivery.smtp_outbound_identifier.to_string())
        .bind(&delivery.smtp_outbound_name)
        .bind(delivery.kind.as_str())
        .bind(delivery.status.as_str())
        .bind(kind)
        .bind(error_message)
        .bind(code)
        .bind(delivery.created_at)
        .bind(waiting.then_some(delivery.created_at))
        .execute(&mut *transaction)
        .await
        .storage()?
        .rows_affected();
        if inserted == 0 {
            continue;
        }
        created += 1;
        for (position, recipient) in delivery.recipients.iter().enumerate() {
            let (kind, error_message, code) = failure_columns(recipient.error.as_ref());
            sqlx::query(
                "INSERT INTO outbound_delivery_recipients(delivery_identifier,position,address,status,error_kind,error_message,error_code) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            )
            .bind(delivery.identifier.to_string())
            .bind(i64::try_from(position).map_err(|_| StorageError::IntegerRange)?)
            .bind(&recipient.address)
            .bind(recipient.status.as_str())
            .bind(kind)
            .bind(error_message)
            .bind(code)
            .execute(&mut *transaction)
            .await
            .storage()?;
        }
    }
    transaction.commit().await.storage()?;
    Ok(created)
}

/**
 * Move deliveries left processing by a stopped process back to retrying.
 */
async fn interrupt_deliveries(
    storage: &SqliteStorage,
    interrupted_at: i64,
) -> Result<u64, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    sqlx::query(
        "UPDATE outbound_delivery_attempts SET finished_at=?1,outcome='interrupted',error_kind='interrupted',error_message=?2 WHERE finished_at IS NULL AND delivery_identifier IN (SELECT identifier FROM outbound_deliveries WHERE status='processing')",
    )
    .bind(interrupted_at)
    .bind(INTERRUPTED_ATTEMPT)
    .execute(&mut *transaction)
    .await
    .storage()?;
    let changed = sqlx::query(
        "UPDATE outbound_deliveries SET status='retrying',next_attempt_at=?1,error_kind='interrupted',error_message=?2,error_code=NULL WHERE status='processing'",
    )
    .bind(interrupted_at)
    .bind(INTERRUPTED_ATTEMPT)
    .execute(&mut *transaction)
    .await
    .storage()?
    .rows_affected();
    transaction.commit().await.storage()?;
    Ok(changed)
}

/**
 * Start an attempt of a due delivery and record it as started.
 */
async fn claim_delivery(
    storage: &SqliteStorage,
    identifier: OutboundDeliveryIdentifier,
    started_at: i64,
) -> Result<Option<OutboundDelivery>, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let claimed = sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE outbound_deliveries SET status='processing',attempt_count=attempt_count+1,last_attempt_at=?2 WHERE identifier=?1 AND status IN {WAITING_STATUSES} AND next_attempt_at<=?2"
    )))
    .bind(identifier.to_string())
    .bind(started_at)
    .execute(&mut *transaction)
    .await
    .storage()?
    .rows_affected();
    if claimed == 0 {
        transaction.commit().await.storage()?;
        return Ok(None);
    }
    sqlx::query(
        "INSERT INTO outbound_delivery_attempts(delivery_identifier,number,started_at) VALUES (?1,(SELECT COALESCE(MAX(number),0)+1 FROM outbound_delivery_attempts WHERE delivery_identifier=?1),?2)",
    )
    .bind(identifier.to_string())
    .bind(started_at)
    .execute(&mut *transaction)
    .await
    .storage()?;
    let delivery = read_delivery(&mut transaction, identifier).await?;
    transaction.commit().await.storage()?;
    Ok(delivery)
}

/**
 * Record the end of a processing delivery's running attempt and apply its new state.
 */
async fn finish_attempt(
    storage: &SqliteStorage,
    identifier: OutboundDeliveryIdentifier,
    result: &OutboundAttemptResult,
) -> Result<bool, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let processing: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM outbound_deliveries WHERE identifier=?1 AND status='processing')",
    )
    .bind(identifier.to_string())
    .fetch_one(&mut *transaction)
    .await
    .storage()?;
    if !processing {
        transaction.commit().await.storage()?;
        return Ok(false);
    }
    let (mut delivered, mut deferred, mut failed) = (0_i64, 0_i64, 0_i64);
    for recipient in &result.recipients {
        match recipient.status {
            OutboundRecipientStatus::Delivered => delivered += 1,
            OutboundRecipientStatus::Pending => deferred += 1,
            OutboundRecipientStatus::Failed => failed += 1,
        }
        let (kind, message, code) = failure_columns(recipient.error.as_ref());
        sqlx::query(
            "UPDATE outbound_delivery_recipients SET status=?3,error_kind=?4,error_message=?5,error_code=?6,response=?7,delivered_at=?8 WHERE delivery_identifier=?1 AND address=?2",
        )
        .bind(identifier.to_string())
        .bind(&recipient.address)
        .bind(recipient.status.as_str())
        .bind(kind)
        .bind(message)
        .bind(code)
        .bind(&recipient.response)
        .bind(recipient.delivered_at)
        .execute(&mut *transaction)
        .await
        .storage()?;
    }
    let (kind, message, code) = failure_columns(result.error.as_ref());
    sqlx::query(
        "UPDATE outbound_delivery_attempts SET finished_at=?2,outcome=?3,error_kind=?4,error_message=?5,error_code=?6,delivered_recipients=?7,deferred_recipients=?8,failed_recipients=?9 WHERE delivery_identifier=?1 AND finished_at IS NULL",
    )
    .bind(identifier.to_string())
    .bind(result.finished_at)
    .bind(result.outcome.as_str())
    .bind(kind)
    .bind(message)
    .bind(code)
    .bind(delivered)
    .bind(deferred)
    .bind(failed)
    .execute(&mut *transaction)
    .await
    .storage()?;
    let delivered_at =
        (result.status == OutboundDeliveryStatus::Delivered).then_some(result.finished_at);
    let next_attempt_at = (result.status == OutboundDeliveryStatus::Retrying)
        .then_some(result.next_attempt_at.unwrap_or(result.finished_at));
    sqlx::query(
        "UPDATE outbound_deliveries SET status=?2,envelope_from=?3,error_kind=?4,error_message=?5,error_code=?6,next_attempt_at=?7,delivered_at=COALESCE(?8,delivered_at) WHERE identifier=?1",
    )
    .bind(identifier.to_string())
    .bind(result.status.as_str())
    .bind(&result.envelope_from)
    .bind(kind)
    .bind(message)
    .bind(code)
    .bind(next_attempt_at)
    .bind(delivered_at)
    .execute(&mut *transaction)
    .await
    .storage()?;
    let attempt_retention =
        i64::try_from(OUTBOUND_ATTEMPT_RETENTION).map_err(|_| StorageError::IntegerRange)?;
    sqlx::query(
        "DELETE FROM outbound_delivery_attempts WHERE delivery_identifier=?1 AND number<=(SELECT number FROM outbound_delivery_attempts WHERE delivery_identifier=?1 ORDER BY number DESC LIMIT 1 OFFSET ?2)",
    )
    .bind(identifier.to_string())
    .bind(attempt_retention)
    .execute(&mut *transaction)
    .await
    .storage()?;
    if result.status.is_terminal() {
        let delivery_retention =
            i64::try_from(OUTBOUND_DELIVERY_RETENTION).map_err(|_| StorageError::IntegerRange)?;
        sqlx::query(
            "DELETE FROM outbound_deliveries WHERE status IN ('delivered','failed') AND sequence<=(SELECT sequence FROM outbound_deliveries WHERE status IN ('delivered','failed') ORDER BY sequence DESC LIMIT 1 OFFSET ?1)",
        )
        .bind(delivery_retention)
        .execute(&mut *transaction)
        .await
        .storage()?;
    }
    transaction.commit().await.storage()?;
    Ok(true)
}

/**
 * Schedule a failed delivery again with its failed recipients pending.
 */
async fn retry_delivery(
    storage: &SqliteStorage,
    identifier: OutboundDeliveryIdentifier,
    retried_at: i64,
) -> Result<Option<OutboundDelivery>, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM outbound_deliveries WHERE identifier=?1")
            .bind(identifier.to_string())
            .fetch_optional(&mut *transaction)
            .await
            .storage()?;
    let Some(status) = status else {
        return Err(StorageError::NotFound);
    };
    if status != OutboundDeliveryStatus::Failed.as_str() {
        transaction.commit().await.storage()?;
        return Ok(None);
    }
    sqlx::query(
        "UPDATE outbound_delivery_recipients SET status='pending',error_kind=NULL,error_message=NULL,error_code=NULL WHERE delivery_identifier=?1 AND status='failed'",
    )
    .bind(identifier.to_string())
    .execute(&mut *transaction)
    .await
    .storage()?;
    sqlx::query(
        "UPDATE outbound_deliveries SET status='retrying',attempt_count=0,next_attempt_at=?2,error_kind=NULL,error_message=NULL,error_code=NULL WHERE identifier=?1",
    )
    .bind(identifier.to_string())
    .bind(retried_at)
    .execute(&mut *transaction)
    .await
    .storage()?;
    let delivery = read_delivery(&mut transaction, identifier).await?;
    transaction.commit().await.storage()?;
    Ok(delivery)
}

/**
 * Read a newest-first page of deliveries with their recipients from one snapshot, applying the
 * optional reader's filter and deletions in SQL before the page limit.
 */
async fn list_deliveries(
    connection: &mut SqliteConnection,
    query: &OutboundDeliveryQuery,
) -> Result<Vec<OutboundDelivery>, StorageError> {
    let mut clauses = Vec::new();
    let mut parameters = Vec::new();
    if let Some(reader) = &query.reader {
        let compiled = compile(&reader.filter);
        clauses.push(format!(
            "({}) AND NOT EXISTS (SELECT 1 FROM user_message_deletions WHERE user_identifier = ? AND mail_sequence = mail.sequence)",
            compiled.sql
        ));
        parameters.extend(compiled.parameters);
        parameters.push(SqlValue::Text(reader.user.to_string()));
    }
    if let Some(message) = query.message {
        clauses.push("delivery.message_identifier = ?".to_owned());
        parameters.push(SqlValue::Text(message.to_string()));
    }
    if let Some(status) = query.status {
        clauses.push("delivery.status = ?".to_owned());
        parameters.push(SqlValue::Text(status.as_str().to_owned()));
    }
    if let Some(before) = query.before {
        clauses.push("delivery.sequence < ?".to_owned());
        parameters.push(SqlValue::Integer(
            i64::try_from(before).map_err(|_| StorageError::IntegerRange)?,
        ));
    }
    parameters.push(SqlValue::Integer(
        i64::try_from(query.limit.min(MAXIMUM_DELIVERY_PAGE))
            .map_err(|_| StorageError::IntegerRange)?,
    ));
    let condition = if clauses.is_empty() {
        "1".to_owned()
    } else {
        clauses.join(" AND ")
    };
    let sql = format!(
        "SELECT {DELIVERY_COLUMNS} FROM outbound_deliveries AS delivery JOIN mail ON mail.identifier=delivery.message_identifier WHERE {condition} ORDER BY delivery.sequence DESC LIMIT ?"
    );
    let mut deliveries = bind_query(sqlx::query(sqlx::AssertSqlSafe(sql)), &parameters)
        .fetch_all(&mut *connection)
        .await
        .storage()?
        .iter()
        .map(decode_delivery)
        .collect::<Result<Vec<_>, _>>()
        .storage()?;
    attach_recipients(connection, &mut deliveries).await?;
    Ok(deliveries)
}

#[async_trait]
impl OutboundStorage for SqliteStorage {
    /**
     * List every SMTP Outbound ordered by name and identifier.
     */
    async fn list_smtp_outbounds(&self) -> Result<Vec<SmtpOutbound>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {OUTBOUND_COLUMNS} FROM smtp_outbounds ORDER BY name COLLATE NOCASE, identifier"
        )))
        .fetch_all(&mut *connection)
        .await
        .storage()?
        .iter()
        .map(decode_outbound)
        .collect::<Result<Vec<_>, _>>()
        .storage()
    }

    /**
     * Load one SMTP Outbound without its password.
     */
    async fn get_smtp_outbound(
        &self,
        identifier: SmtpOutboundIdentifier,
    ) -> Result<Option<SmtpOutbound>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        read_outbound(&mut connection, identifier).await
    }

    /**
     * Load one SMTP Outbound and its encrypted password from the same row.
     */
    async fn get_smtp_outbound_credential(
        &self,
        identifier: SmtpOutboundIdentifier,
    ) -> Result<Option<SmtpOutboundCredential>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        let Some(row) = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {OUTBOUND_COLUMNS},encrypted_password FROM smtp_outbounds WHERE identifier=?1"
        )))
        .bind(identifier.to_string())
        .fetch_optional(&mut *connection)
        .await
        .storage()?
        else {
            return Ok(None);
        };
        Ok(Some(SmtpOutboundCredential {
            outbound: decode_outbound(&row).storage()?,
            encrypted_password: row.try_get("encrypted_password").storage()?,
        }))
    }

    /**
     * Create an SMTP Outbound with a case-insensitively unique name.
     */
    async fn create_smtp_outbound(
        &self,
        outbound: &NewSmtpOutbound,
    ) -> Result<SmtpOutbound, StorageError> {
        create_outbound(self, outbound).await
    }

    /**
     * Replace an SMTP Outbound's settings and apply the password change.
     */
    async fn update_smtp_outbound(
        &self,
        identifier: SmtpOutboundIdentifier,
        settings: &SmtpOutboundSettings,
        password: &SmtpOutboundPassword,
        updated_at: i64,
    ) -> Result<Option<SmtpOutbound>, StorageError> {
        update_outbound(self, identifier, settings, password, updated_at).await
    }

    /**
     * Delete an unreferenced SMTP Outbound.
     */
    async fn delete_smtp_outbound(
        &self,
        identifier: SmtpOutboundIdentifier,
    ) -> Result<bool, StorageError> {
        delete_outbound(self, identifier).await
    }

    /**
     * List routing actions, optionally only those of one scope.
     */
    async fn list_routing_actions(
        &self,
        scope: Option<ScopeIdentifier>,
    ) -> Result<Vec<RoutingAction>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {ACTION_COLUMNS} FROM scope_routing_actions WHERE ?1 IS NULL OR scope_identifier=?1 ORDER BY scope_identifier, created_at, rowid"
        )))
        .bind(scope.map(|scope| scope.to_string()))
        .fetch_all(&mut *connection)
        .await
        .storage()?
        .iter()
        .map(decode_action)
        .collect::<Result<Vec<_>, _>>()
        .storage()
    }

    /**
     * Load one routing action.
     */
    async fn get_routing_action(
        &self,
        identifier: RoutingActionIdentifier,
    ) -> Result<Option<RoutingAction>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        read_action(&mut connection, identifier).await
    }

    /**
     * Create a routing action for an existing scope and SMTP Outbound.
     */
    async fn create_routing_action(
        &self,
        action: &NewRoutingAction,
    ) -> Result<RoutingAction, StorageError> {
        create_action(self, action).await
    }

    /**
     * Replace a routing action's mutable fields.
     */
    async fn update_routing_action(
        &self,
        identifier: RoutingActionIdentifier,
        update: &RoutingActionUpdate,
        updated_at: i64,
    ) -> Result<Option<RoutingAction>, StorageError> {
        update_action(self, identifier, update, updated_at).await
    }

    /**
     * Delete a routing action.
     */
    async fn delete_routing_action(
        &self,
        identifier: RoutingActionIdentifier,
    ) -> Result<bool, StorageError> {
        delete_action(self, identifier).await
    }

    /**
     * List queued messages in queue order.
     */
    async fn list_routing_evaluations(
        &self,
        limit: usize,
    ) -> Result<Vec<MessageIdentifier>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        let identifiers: Vec<String> = sqlx::query_scalar(
            "SELECT message_identifier FROM mail_routing_evaluations ORDER BY sequence LIMIT ?1",
        )
        .bind(i64::try_from(limit).map_err(|_| StorageError::IntegerRange)?)
        .fetch_all(&mut *connection)
        .await
        .storage()?;
        identifiers
            .iter()
            .map(|identifier| parse_identifier(identifier))
            .collect()
    }

    /**
     * Create a queued message's deliveries and dequeue it atomically.
     */
    async fn complete_routing_evaluation(
        &self,
        message: MessageIdentifier,
        deliveries: &[NewOutboundDelivery],
    ) -> Result<u64, StorageError> {
        complete_evaluation(self, message, deliveries).await
    }

    /**
     * Return deliveries interrupted by a stopped process to the retry queue.
     */
    async fn interrupt_outbound_deliveries(
        &self,
        interrupted_at: i64,
    ) -> Result<u64, StorageError> {
        interrupt_deliveries(self, interrupted_at).await
    }

    /**
     * List due waiting deliveries, earliest due first.
     */
    async fn list_due_outbound_deliveries(
        &self,
        now: i64,
        limit: usize,
    ) -> Result<Vec<OutboundDelivery>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        let mut deliveries = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {DELIVERY_COLUMNS} FROM outbound_deliveries AS delivery JOIN mail ON mail.identifier=delivery.message_identifier WHERE delivery.status IN {WAITING_STATUSES} AND delivery.next_attempt_at<=?1 ORDER BY delivery.next_attempt_at, delivery.sequence LIMIT ?2"
        )))
        .bind(now)
        .bind(i64::try_from(limit).map_err(|_| StorageError::IntegerRange)?)
        .fetch_all(&mut *connection)
        .await
        .storage()?
        .iter()
        .map(decode_delivery)
        .collect::<Result<Vec<_>, _>>()
        .storage()?;
        attach_recipients(&mut connection, &mut deliveries).await?;
        Ok(deliveries)
    }

    /**
     * Claim a due delivery for an attempt.
     */
    async fn claim_outbound_delivery(
        &self,
        identifier: OutboundDeliveryIdentifier,
        started_at: i64,
    ) -> Result<Option<OutboundDelivery>, StorageError> {
        claim_delivery(self, identifier, started_at).await
    }

    /**
     * Record the end of a processing delivery's attempt.
     */
    async fn finish_outbound_delivery_attempt(
        &self,
        identifier: OutboundDeliveryIdentifier,
        result: &OutboundAttemptResult,
    ) -> Result<bool, StorageError> {
        finish_attempt(self, identifier, result).await
    }

    /**
     * Schedule a failed delivery again.
     */
    async fn retry_outbound_delivery(
        &self,
        identifier: OutboundDeliveryIdentifier,
        retried_at: i64,
    ) -> Result<Option<OutboundDelivery>, StorageError> {
        retry_delivery(self, identifier, retried_at).await
    }

    /**
     * List deliveries newest first from one read transaction.
     */
    async fn list_outbound_deliveries(
        &self,
        query: &OutboundDeliveryQuery,
    ) -> Result<Vec<OutboundDelivery>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let deliveries = list_deliveries(&mut transaction, query).await?;
        transaction.commit().await.storage()?;
        Ok(deliveries)
    }

    /**
     * Load one delivery with its recipients and attempts from one read transaction.
     */
    async fn get_outbound_delivery(
        &self,
        identifier: OutboundDeliveryIdentifier,
    ) -> Result<Option<(OutboundDelivery, Vec<OutboundDeliveryAttempt>)>, StorageError> {
        let mut transaction = self.read_pool.begin().await.storage()?;
        let Some(delivery) = read_delivery(&mut transaction, identifier).await? else {
            transaction.commit().await.storage()?;
            return Ok(None);
        };
        let attempts = sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {ATTEMPT_COLUMNS} FROM outbound_delivery_attempts WHERE delivery_identifier=?1 ORDER BY number"
        )))
        .bind(identifier.to_string())
        .fetch_all(&mut *transaction)
        .await
        .storage()?
        .iter()
        .map(decode_attempt)
        .collect::<Result<Vec<_>, _>>()
        .storage()?;
        transaction.commit().await.storage()?;
        Ok(Some((delivery, attempts)))
    }
}
