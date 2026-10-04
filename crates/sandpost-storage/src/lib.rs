use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sandpost_core::{Message, MessageIdentifier, MessageSequence, Scope, ScopeIdentifier};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

const SCHEMA_VERSION: i64 = 2;
const MAXIMUM_PAGE_SIZE: usize = 100;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    #[error("unsupported database schema version {0} (max supported {SCHEMA_VERSION})")]
    NewerSchema(i64),
    #[error("scope {0} has a different policy version")]
    StalePolicy(ScopeIdentifier),
    #[error("message {0} already exists")]
    DuplicateMessageIdentifier(MessageIdentifier),
    #[error("scope {0} policy versions must increase when its filter or parent changes")]
    PolicyVersionConflict(ScopeIdentifier),
    #[error("integer value is outside SQLite's supported range")]
    IntegerRange,
    #[error("storage connection lock was poisoned")]
    LockPoisoned,
    #[error("invalid stored value: {0}")]
    InvalidData(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSummary {
    #[serde(rename = "seq")]
    pub sequence: MessageSequence,
    #[serde(rename = "id")]
    pub identifier: MessageIdentifier,
    pub subject: String,
    pub from: Vec<sandpost_core::Mailbox>,
    pub to: Vec<sandpost_core::Mailbox>,
    pub received_at: i64,
    pub size: u64,
    pub attachment_count: u64,
}

// ponytail: one connection-wide lock; keep calls on spawn_blocking and split connections only if contention matters.
#[derive(Clone)]
pub struct Storage(Arc<Mutex<Connection>>);

impl Storage {
    /// Open or create a database at the supplied path and apply pending migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let connection = Connection::open(path)?;
        Self::initialize(connection)
    }

    /// Create an in-memory database and apply the current schema.
    pub fn memory() -> Result<Self, StorageError> {
        Self::initialize(Connection::open_in_memory()?)
    }

    /// Configure the connection, migrate its schema, and wrap it for shared access.
    fn initialize(connection: Connection) -> Result<Self, StorageError> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        migrate(&connection)?;
        Ok(Self(Arc::new(Mutex::new(connection))))
    }

    /// Lock the shared database connection, reporting a poisoned lock as a storage error.
    fn connection(&self) -> Result<MutexGuard<'_, Connection>, StorageError> {
        self.0.lock().map_err(|_| StorageError::LockPoisoned)
    }

    /// Check that the database can execute a trivial query.
    pub fn health(&self) -> Result<(), StorageError> {
        self.connection()?.query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }

    /// Insert a message once and materialize its current scope matches atomically.
    pub fn insert_message(
        &self,
        message: &Message,
        matches: &[(ScopeIdentifier, u64)],
    ) -> Result<MessageSequence, StorageError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let facts = serde_json::to_string(&message.facts)?;
        let attachments = serde_json::to_string(&message.attachments)?;
        let sender_domain = message
            .facts
            .envelope_from
            .as_ref()
            .or_else(|| message.facts.from.first())
            .map(|mailbox| mailbox.domain.as_str())
            .unwrap_or("");
        let inserted = transaction.execute(
            "INSERT INTO messages(id, facts, raw_mime, attachments, sender_domain, received_at, subject, from_json, to_json, size, attachment_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                message.identifier.to_string(), facts, message.raw_message, attachments, sender_domain,
                message.facts.received_at, message.facts.subject,
                serde_json::to_string(&message.facts.from)?, serde_json::to_string(&message.facts.to)?,
                i64::try_from(message.facts.size).map_err(|_| StorageError::IntegerRange)?,
                i64::try_from(message.facts.attachment_count).map_err(|_| StorageError::IntegerRange)?,
            ],
        );
        let inserted = match inserted {
            Ok(changes) => changes,
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(StorageError::DuplicateMessageIdentifier(message.identifier));
            }
            Err(error) => return Err(error.into()),
        };
        let stored_sequence: i64 = transaction.query_row(
            "SELECT seq FROM messages WHERE id = ?1",
            [message.identifier.to_string()],
            |row| row.get(0),
        )?;
        let sequence = MessageSequence(
            u64::try_from(stored_sequence).map_err(|_| StorageError::IntegerRange)?,
        );
        if inserted > 0 {
            index_message(&transaction, stored_sequence, message)?;
        }
        for (scope_identifier, policy_version) in matches {
            let current: Option<i64> = transaction
                .query_row(
                    "SELECT policy_version FROM scopes WHERE id = ?1",
                    [scope_identifier.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(current) = current else {
                return Err(rusqlite::Error::QueryReturnedNoRows.into());
            };
            if u64::try_from(current).ok() != Some(*policy_version) {
                return Err(StorageError::StalePolicy(*scope_identifier));
            }
            transaction.execute(
                "INSERT INTO message_scope(scope_id, message_seq, policy_version) VALUES (?1, ?2, ?3) ON CONFLICT(scope_id, message_seq) DO UPDATE SET policy_version = excluded.policy_version",
                params![scope_identifier.to_string(), stored_sequence, i64::try_from(*policy_version).map_err(|_| StorageError::IntegerRange)?],
            )?;
        }
        transaction.commit()?;
        Ok(sequence)
    }

    /// List messages newest first, optionally restricting results to sequences before a cursor.
    pub fn list_messages(
        &self,
        before: Option<MessageSequence>,
        limit: usize,
    ) -> Result<Vec<MessageSummary>, StorageError> {
        let connection = self.connection()?;
        let limit = limit.min(MAXIMUM_PAGE_SIZE);
        let mut statement = connection.prepare_cached(
            "SELECT seq, id, subject, from_json, to_json, received_at, size, attachment_count FROM messages WHERE (?1 IS NULL OR seq < ?1) ORDER BY seq DESC LIMIT ?2",
        )?;
        let before = before
            .map(|sequence| i64::try_from(sequence.0).map_err(|_| StorageError::IntegerRange))
            .transpose()?;
        let rows = statement.query_map(params![before, limit as i64], summary_row)?;
        rows.map(|database_row| {
            database_row
                .map_err(StorageError::from)
                .and_then(decode_summary)
        })
        .collect()
    }

    /// List current matches from these scopes. Scope count is bounded by SQLite's bind limit.
    pub fn list_visible_messages(
        &self,
        scope_identifiers: &[ScopeIdentifier],
        before: Option<MessageSequence>,
        limit: usize,
    ) -> Result<Vec<MessageSummary>, StorageError> {
        if scope_identifiers.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let connection = self.connection()?;
        let limit = limit.min(MAXIMUM_PAGE_SIZE);
        let scope_placeholders = (2..scope_identifiers.len() + 2)
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let query_statement = format!(
            "WITH visible AS MATERIALIZED (\
                SELECT ms.message_seq FROM message_scope ms \
                JOIN scopes s ON s.id = ms.scope_id AND ms.policy_version = s.policy_version \
                WHERE ms.scope_id IN ({scope_placeholders}) AND (?1 IS NULL OR ms.message_seq < ?1) \
                GROUP BY ms.message_seq ORDER BY ms.message_seq DESC LIMIT ?{}\
             ) SELECT m.seq, m.id, m.subject, m.from_json, m.to_json, m.received_at, m.size, m.attachment_count \
             FROM visible JOIN messages m ON m.seq = visible.message_seq ORDER BY m.seq DESC",
            scope_identifiers.len() + 2,
        );
        let mut statement = connection.prepare_cached(&query_statement)?;
        let mut values = Vec::with_capacity(scope_identifiers.len() + 2);
        let before = before
            .map(|sequence| i64::try_from(sequence.0).map_err(|_| StorageError::IntegerRange))
            .transpose()?;
        values.push(before.map_or(
            rusqlite::types::Value::Null,
            rusqlite::types::Value::Integer,
        ));
        values.extend(
            scope_identifiers
                .iter()
                .map(|identifier| rusqlite::types::Value::Text(identifier.to_string())),
        );
        values.push(rusqlite::types::Value::Integer(limit as i64));
        let rows = statement.query_map(rusqlite::params_from_iter(values), summary_row)?;
        rows.map(|database_row| {
            database_row
                .map_err(StorageError::from)
                .and_then(decode_summary)
        })
        .collect()
    }

    /// Load a message, including its original raw message bytes, by identifier.
    pub fn get_message(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<Message>, StorageError> {
        let connection = self.connection()?;
        let stored: Option<(String, Vec<u8>, String)> = connection
            .query_row(
                "SELECT facts, raw_mime, attachments FROM messages WHERE id = ?1",
                [identifier.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        stored
            .map(|(facts, raw_message, attachments)| {
                Ok(Message {
                    identifier,
                    facts: serde_json::from_str(&facts)?,
                    raw_message,
                    attachments: serde_json::from_str(&attachments)?,
                })
            })
            .transpose()
    }

    /// Read parsed message data and attachment metadata without fetching raw MIME bytes.
    pub fn get_message_metadata(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<(sandpost_core::MessageFacts, Vec<sandpost_core::Attachment>)>, StorageError>
    {
        let connection = self.connection()?;
        let stored: Option<(String, String)> = connection
            .query_row(
                "SELECT facts, attachments FROM messages WHERE id = ?1",
                [identifier.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        stored
            .map(|(facts, attachments)| {
                Ok((
                    serde_json::from_str(&facts)?,
                    serde_json::from_str(&attachments)?,
                ))
            })
            .transpose()
    }

    /// Save a scope while enforcing policy-version changes for filter or parent updates.
    pub fn save_scope(&self, scope: &Scope) -> Result<(), StorageError> {
        let version =
            i64::try_from(scope.policy_version).map_err(|_| StorageError::IntegerRange)?;
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let previous: Option<(String, Option<String>, i64)> = transaction
            .query_row(
                "SELECT filter, parent_id, policy_version FROM scopes WHERE id=?1",
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
            "INSERT INTO scopes(id, parent_id, name, description, filter, position, policy_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(id) DO UPDATE SET parent_id=excluded.parent_id, name=excluded.name, description=excluded.description, filter=excluded.filter, position=excluded.position, policy_version=excluded.policy_version",
            params![scope.identifier.to_string(), scope.parent.map(|identifier| identifier.to_string()), scope.name, scope.description, scope.filter, scope.position, version],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Load scopes ordered by their configured position and identifier.
    pub fn load_scopes(&self) -> Result<Vec<Scope>, StorageError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare_cached("SELECT id, parent_id, name, description, filter, position, policy_version FROM scopes ORDER BY position, id")?;
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

    /// Replace one scope's complete match set only when its policy version is current.
    pub fn replace_scope_matches(
        &self,
        scope_identifier: ScopeIdentifier,
        policy_version: u64,
        message_sequences: &[MessageSequence],
    ) -> Result<(), StorageError> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let current: Option<i64> = transaction
            .query_row(
                "SELECT policy_version FROM scopes WHERE id=?1",
                [scope_identifier.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if current.and_then(|value| u64::try_from(value).ok()) != Some(policy_version) {
            return Err(StorageError::StalePolicy(scope_identifier));
        }
        transaction.execute(
            "DELETE FROM message_scope WHERE scope_id=?1",
            [scope_identifier.to_string()],
        )?;
        {
            let mut statement = transaction.prepare_cached("INSERT INTO message_scope(scope_id, message_seq, policy_version) VALUES (?1, ?2, ?3)")?;
            for sequence in message_sequences {
                statement.execute(params![
                    scope_identifier.to_string(),
                    i64::try_from(sequence.0).map_err(|_| StorageError::IntegerRange)?,
                    i64::try_from(policy_version).map_err(|_| StorageError::IntegerRange)?
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }
}

/// Index a message's recipient addresses and ordered headers inside its insert transaction.
fn index_message(
    transaction: &Transaction<'_>,
    sequence: i64,
    message: &Message,
) -> Result<(), StorageError> {
    let mut recipients = transaction.prepare_cached("INSERT OR IGNORE INTO message_recipients(message_seq, address, domain) VALUES (?1, ?2, ?3)")?;
    for mailbox in message
        .facts
        .envelope_to
        .iter()
        .chain(&message.facts.to)
        .chain(&message.facts.carbon_copy)
    {
        recipients.execute(params![sequence, mailbox.address, mailbox.domain])?;
    }
    drop(recipients);
    let mut headers = transaction.prepare_cached(
        "INSERT INTO message_headers(message_seq, name, value, ordinal) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (name, values) in &message.facts.headers {
        for (ordinal, value) in values.iter().enumerate() {
            headers.execute(params![
                sequence,
                name,
                value,
                i64::try_from(ordinal).map_err(|_| StorageError::IntegerRange)?
            ])?;
        }
    }
    Ok(())
}

/// Apply missing schema migrations atomically and reject databases from newer versions.
fn migrate(connection: &Connection) -> Result<(), StorageError> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(StorageError::NewerSchema(version));
    }
    if version < SCHEMA_VERSION {
        connection.execute_batch("BEGIN IMMEDIATE;")?;
        let result = (|| {
            let mut current: i64 =
                connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
            if current > SCHEMA_VERSION {
                return Err(StorageError::NewerSchema(current));
            }
            while current < SCHEMA_VERSION {
                let next = current + 1;
                let migration = match next {
                    1 => include_str!("../migrations/0001_initial.sql"),
                    2 => include_str!("../migrations/0002_message_summaries.sql"),
                    _ => {
                        return Err(StorageError::InvalidData(format!(
                            "missing migration {next}"
                        )));
                    }
                };
                connection.execute_batch(migration)?;
                connection.pragma_update(None, "user_version", next)?;
                current = next;
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = connection.execute_batch("ROLLBACK;");
            return Err(error);
        }
        connection.execute_batch("COMMIT;")?;
    }
    Ok(())
}

type StoredSummary = (i64, String, String, String, String, i64, i64, i64);

/// Read the selected summary columns in their stable query order.
fn summary_row(database_row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSummary> {
    Ok((
        database_row.get(0)?,
        database_row.get(1)?,
        database_row.get(2)?,
        database_row.get(3)?,
        database_row.get(4)?,
        database_row.get(5)?,
        database_row.get(6)?,
        database_row.get(7)?,
    ))
}

/// Convert a stored summary row to domain values, validating numeric and serialized fields.
fn decode_summary(
    (sequence, identifier, subject, from, to, received_at, size, attachment_count): StoredSummary,
) -> Result<MessageSummary, StorageError> {
    Ok(MessageSummary {
        sequence: MessageSequence(u64::try_from(sequence).map_err(|_| StorageError::IntegerRange)?),
        identifier: parse_identifier(&identifier)?,
        subject,
        from: serde_json::from_str(&from)?,
        to: serde_json::from_str(&to)?,
        received_at,
        size: u64::try_from(size).map_err(|_| StorageError::IntegerRange)?,
        attachment_count: u64::try_from(attachment_count)
            .map_err(|_| StorageError::IntegerRange)?,
    })
}

/// Parse a stored identifier and preserve the invalid source value in the error.
fn parse_identifier<IdentifierType: std::str::FromStr>(
    value: &str,
) -> Result<IdentifierType, StorageError> {
    value
        .parse()
        .map_err(|_| StorageError::InvalidData(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandpost_core::{Attachment, Mailbox, MessageFacts};

    /// Create a representative message with sender, recipient, and attachment data.
    fn message() -> Message {
        Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                envelope_from: Some(Mailbox {
                    address: "sender@example.org".into(),
                    domain: "example.org".into(),
                }),
                from: vec![Mailbox {
                    address: "sender@example.org".into(),
                    domain: "example.org".into(),
                }],
                to: vec![Mailbox {
                    address: "reader@example.net".into(),
                    domain: "example.net".into(),
                }],
                subject: "hello".into(),
                text: "body".into(),
                received_at: 123,
                size: 99,
                attachment_count: 1,
                headers: [("x-tag".into(), vec!["one".into(), "two".into()])].into(),
                ..Default::default()
            },
            raw_message: b"raw".to_vec(),
            attachments: vec![Attachment {
                filename: Some("a.bin".into()),
                content_type: "application/octet-stream".into(),
                size: 3,
                content_hash: "hash".into(),
            }],
        }
    }
    /// Create a root scope with the requested policy version.
    fn scope(policy_version: u64) -> Scope {
        Scope {
            identifier: ScopeIdentifier::new(),
            parent: None,
            name: "root".into(),
            description: None,
            filter: String::new(),
            position: 0,
            policy_version,
        }
    }

    /// Verify migrations can reopen a database and reject a newer schema version.
    #[test]
    fn migration_is_idempotent_across_reopen_and_schema_must_not_be_newer() {
        let path = std::env::temp_dir().join(format!("sandpost-{}.db", MessageIdentifier::new()));
        Storage::open(&path).unwrap();
        Storage::open(&path).unwrap().health().unwrap();
        let connection = Connection::open(&path).unwrap();
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        drop(connection);
        assert!(matches!(
            Storage::open(&path),
            Err(StorageError::NewerSchema(3))
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    /// Verify migration from the first schema fills in message summary columns.
    #[test]
    fn version_one_database_backfills_narrow_summary_columns() {
        let path =
            std::env::temp_dir().join(format!("sandpost-v1-{}.db", MessageIdentifier::new()));
        let original = message();
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(include_str!("../migrations/0001_initial.sql"))
            .unwrap();
        connection.execute(
            "INSERT INTO messages(id, facts, raw_mime, attachments, sender_domain, received_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                original.identifier.to_string(),
                serde_json::to_string(&original.facts).unwrap(),
                original.raw_message,
                serde_json::to_string(&original.attachments).unwrap(),
                "example.org",
                original.facts.received_at,
            ],
        )
        .unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        drop(connection);

        let storage = Storage::open(&path).unwrap();
        let summary = storage.list_messages(None, 10).unwrap().remove(0);
        assert_eq!(summary.subject, original.facts.subject);
        assert_eq!(summary.from, original.facts.from);
        assert_eq!(summary.to, original.facts.to);
        assert_eq!(summary.size, original.facts.size);
        drop(storage);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    /// Verify message insertion, retrieval, metadata decoding, and duplicate rejection.
    #[test]
    fn ingest_roundtrip_metadata_and_duplicate_rejection() {
        let storage = Storage::memory().unwrap();
        let original = message();
        let sequence = storage.insert_message(&original, &[]).unwrap();
        let mut duplicate = original.clone();
        duplicate.raw_message = b"replacement".to_vec();
        assert!(matches!(
            storage.insert_message(&duplicate, &[]),
            Err(StorageError::DuplicateMessageIdentifier(identifier)) if identifier == original.identifier
        ));
        let saved = storage.get_message(original.identifier).unwrap().unwrap();
        assert_eq!(saved.raw_message, b"raw");
        assert_eq!(saved.facts, original.facts);
        assert_eq!(saved.attachments, original.attachments);
        assert_eq!(
            storage
                .get_message_metadata(original.identifier)
                .unwrap()
                .unwrap(),
            (original.facts.clone(), original.attachments.clone())
        );
        assert_eq!(
            storage.list_messages(None, 10).unwrap()[0].sequence,
            sequence
        );
    }

    /// Verify a foreign-key failure rolls back the message and its scope matches.
    #[test]
    fn foreign_key_failure_rolls_back_message_and_scope_matches() {
        let storage = Storage::memory().unwrap();
        let message = message();
        let unknown = ScopeIdentifier::new();
        assert!(storage.insert_message(&message, &[(unknown, 1)]).is_err());
        assert!(storage.get_message(message.identifier).unwrap().is_none());
    }

    /// Verify pagination, match materialization, scope unions, and policy version guards.
    #[test]
    fn pagination_scope_materialization_union_and_policy_guard() {
        let storage = Storage::memory().unwrap();
        let scope_one = scope(3);
        let scope_two = scope(8);
        storage.save_scope(&scope_one).unwrap();
        storage.save_scope(&scope_two).unwrap();
        assert_eq!(storage.load_scopes().unwrap().len(), 2);
        let first_message = message();
        let first_sequence = storage
            .insert_message(
                &first_message,
                &[(scope_one.identifier, 3), (scope_two.identifier, 8)],
            )
            .unwrap();
        let mut second_message = message();
        second_message.identifier = MessageIdentifier::new();
        let second_sequence = storage
            .insert_message(&second_message, &[(scope_one.identifier, 3)])
            .unwrap();
        let connection = storage.connection().unwrap();
        let count: i64 = connection
            .query_row("SELECT count(*) FROM message_scope", [], |database_row| {
                database_row.get(0)
            })
            .unwrap();
        assert_eq!(count, 3);
        drop(connection);
        assert_eq!(
            storage.list_messages(None, 1).unwrap()[0].sequence,
            second_sequence
        );
        assert_eq!(
            storage.list_messages(Some(second_sequence), 10).unwrap()[0].sequence,
            first_sequence
        );
        let visible = storage
            .list_visible_messages(&[scope_one.identifier], None, 10)
            .unwrap();
        assert_eq!(
            visible.iter().map(|row| row.sequence).collect::<Vec<_>>(),
            vec![second_sequence, first_sequence]
        );
        assert!(
            matches!(storage.replace_scope_matches(scope_one.identifier, 2, &[first_sequence]), Err(StorageError::StalePolicy(identifier)) if identifier == scope_one.identifier)
        );
        storage
            .replace_scope_matches(scope_one.identifier, 3, &[first_sequence])
            .unwrap();
        let connection = storage.connection().unwrap();
        let rows: i64 = connection
            .query_row(
                "SELECT count(*) FROM message_scope WHERE scope_id=?1 AND policy_version=3",
                [scope_one.identifier.to_string()],
                |database_row| database_row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1);
        drop(connection);
        let mut changed = scope_one.clone();
        changed.filter = "subject contains 'changed'".into();
        assert!(
            matches!(storage.save_scope(&changed), Err(StorageError::PolicyVersionConflict(identifier)) if identifier == scope_one.identifier)
        );
        changed.policy_version = 4;
        storage.save_scope(&changed).unwrap();
        assert!(
            storage
                .list_visible_messages(&[scope_one.identifier], None, 10)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            storage
                .list_visible_messages(&[scope_one.identifier, scope_two.identifier], None, 10)
                .unwrap()
                .len(),
            1
        );
        let mut reparented = changed.clone();
        reparented.parent = Some(scope_two.identifier);
        assert!(matches!(
            storage.save_scope(&reparented),
            Err(StorageError::PolicyVersionConflict(identifier)) if identifier == scope_one.identifier
        ));
        storage
            .replace_scope_matches(scope_one.identifier, 4, &[first_sequence])
            .unwrap();
        assert_eq!(
            storage
                .list_visible_messages(&[scope_one.identifier], None, 10)
                .unwrap()
                .len(),
            1
        );
    }
}
