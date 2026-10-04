use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sandpost_core::{Message, MessageId, MessageSeq, Scope, ScopeId};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

const SCHEMA_VERSION: i64 = 2;
const MAX_PAGE_SIZE: usize = 100;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("unsupported database schema version {0} (max supported {SCHEMA_VERSION})")]
    NewerSchema(i64),
    #[error("scope {0} has a different policy version")]
    StalePolicy(ScopeId),
    #[error("message {0} already exists")]
    DuplicateMessageId(MessageId),
    #[error("scope {0} policy versions must increase when its filter or parent changes")]
    PolicyVersionConflict(ScopeId),
    #[error("integer value is outside SQLite's supported range")]
    IntegerRange,
    #[error("storage connection lock was poisoned")]
    LockPoisoned,
    #[error("invalid stored value: {0}")]
    InvalidData(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSummary {
    pub seq: MessageSeq,
    pub id: MessageId,
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
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let connection = Connection::open(path)?;
        Self::initialize(connection)
    }

    pub fn memory() -> Result<Self, StorageError> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(connection: Connection) -> Result<Self, StorageError> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        migrate(&connection)?;
        Ok(Self(Arc::new(Mutex::new(connection))))
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, StorageError> {
        self.0.lock().map_err(|_| StorageError::LockPoisoned)
    }

    pub fn health(&self) -> Result<(), StorageError> {
        self.connection()?.query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }

    /// Insert a message once and materialize its current scope matches atomically.
    pub fn insert_message(
        &self,
        message: &Message,
        matches: &[(ScopeId, u64)],
    ) -> Result<MessageSeq, StorageError> {
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let facts = serde_json::to_string(&message.facts)?;
        let attachments = serde_json::to_string(&message.attachments)?;
        let sender_domain = message
            .facts
            .envelope_from
            .as_ref()
            .or_else(|| message.facts.from.first())
            .map(|m| m.domain.as_str())
            .unwrap_or("");
        let inserted = tx.execute(
            "INSERT INTO messages(id, facts, raw_mime, attachments, sender_domain, received_at, subject, from_json, to_json, size, attachment_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                message.id.to_string(), facts, message.raw_mime, attachments, sender_domain,
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
                return Err(StorageError::DuplicateMessageId(message.id));
            }
            Err(error) => return Err(error.into()),
        };
        let raw_seq: i64 = tx.query_row(
            "SELECT seq FROM messages WHERE id = ?1",
            [message.id.to_string()],
            |row| row.get(0),
        )?;
        let seq = MessageSeq(u64::try_from(raw_seq).map_err(|_| StorageError::IntegerRange)?);
        if inserted > 0 {
            index_message(&tx, raw_seq, message)?;
        }
        for (scope_id, version) in matches {
            let current: Option<i64> = tx
                .query_row(
                    "SELECT policy_version FROM scopes WHERE id = ?1",
                    [scope_id.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(current) = current else {
                return Err(rusqlite::Error::QueryReturnedNoRows.into());
            };
            if u64::try_from(current).ok() != Some(*version) {
                return Err(StorageError::StalePolicy(*scope_id));
            }
            tx.execute(
                "INSERT INTO message_scope(scope_id, message_seq, policy_version) VALUES (?1, ?2, ?3) ON CONFLICT(scope_id, message_seq) DO UPDATE SET policy_version = excluded.policy_version",
                params![scope_id.to_string(), raw_seq, i64::try_from(*version).map_err(|_| StorageError::IntegerRange)?],
            )?;
        }
        tx.commit()?;
        Ok(seq)
    }

    pub fn list_messages(
        &self,
        before: Option<MessageSeq>,
        limit: usize,
    ) -> Result<Vec<MessageSummary>, StorageError> {
        let connection = self.connection()?;
        let limit = limit.min(MAX_PAGE_SIZE);
        let mut statement = connection.prepare_cached(
            "SELECT seq, id, subject, from_json, to_json, received_at, size, attachment_count FROM messages WHERE (?1 IS NULL OR seq < ?1) ORDER BY seq DESC LIMIT ?2",
        )?;
        let before = before
            .map(|seq| i64::try_from(seq.0).map_err(|_| StorageError::IntegerRange))
            .transpose()?;
        let rows = statement.query_map(params![before, limit as i64], summary_row)?;
        rows.map(|row| row.map_err(StorageError::from).and_then(decode_summary))
            .collect()
    }

    /// List current matches from these scopes. Scope count is bounded by SQLite's bind limit.
    pub fn list_visible_messages(
        &self,
        scope_ids: &[ScopeId],
        before: Option<MessageSeq>,
        limit: usize,
    ) -> Result<Vec<MessageSummary>, StorageError> {
        if scope_ids.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let connection = self.connection()?;
        let limit = limit.min(MAX_PAGE_SIZE);
        let scope_placeholders = (2..scope_ids.len() + 2)
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "WITH visible AS MATERIALIZED (\
                SELECT ms.message_seq FROM message_scope ms \
                JOIN scopes s ON s.id = ms.scope_id AND ms.policy_version = s.policy_version \
                WHERE ms.scope_id IN ({scope_placeholders}) AND (?1 IS NULL OR ms.message_seq < ?1) \
                GROUP BY ms.message_seq ORDER BY ms.message_seq DESC LIMIT ?{}\
             ) SELECT m.seq, m.id, m.subject, m.from_json, m.to_json, m.received_at, m.size, m.attachment_count \
             FROM visible JOIN messages m ON m.seq = visible.message_seq ORDER BY m.seq DESC",
            scope_ids.len() + 2,
        );
        let mut statement = connection.prepare_cached(&sql)?;
        let mut values = Vec::with_capacity(scope_ids.len() + 2);
        let before = before
            .map(|seq| i64::try_from(seq.0).map_err(|_| StorageError::IntegerRange))
            .transpose()?;
        values.push(before.map_or(
            rusqlite::types::Value::Null,
            rusqlite::types::Value::Integer,
        ));
        values.extend(
            scope_ids
                .iter()
                .map(|id| rusqlite::types::Value::Text(id.to_string())),
        );
        values.push(rusqlite::types::Value::Integer(limit as i64));
        let rows = statement.query_map(rusqlite::params_from_iter(values), summary_row)?;
        rows.map(|row| row.map_err(StorageError::from).and_then(decode_summary))
            .collect()
    }

    pub fn get_message(&self, id: MessageId) -> Result<Option<Message>, StorageError> {
        let connection = self.connection()?;
        let stored: Option<(String, Vec<u8>, String)> = connection
            .query_row(
                "SELECT facts, raw_mime, attachments FROM messages WHERE id = ?1",
                [id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        stored
            .map(|(facts, raw_mime, attachments)| {
                Ok(Message {
                    id,
                    facts: serde_json::from_str(&facts)?,
                    raw_mime,
                    attachments: serde_json::from_str(&attachments)?,
                })
            })
            .transpose()
    }

    /// Read parsed message data and attachment metadata without fetching raw MIME bytes.
    pub fn get_message_metadata(
        &self,
        id: MessageId,
    ) -> Result<Option<(sandpost_core::MessageFacts, Vec<sandpost_core::Attachment>)>, StorageError>
    {
        let connection = self.connection()?;
        let stored: Option<(String, String)> = connection
            .query_row(
                "SELECT facts, attachments FROM messages WHERE id = ?1",
                [id.to_string()],
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

    pub fn save_scope(&self, scope: &Scope) -> Result<(), StorageError> {
        let version =
            i64::try_from(scope.policy_version).map_err(|_| StorageError::IntegerRange)?;
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let previous: Option<(String, Option<String>, i64)> = tx
            .query_row(
                "SELECT filter, parent_id, policy_version FROM scopes WHERE id=?1",
                [scope.id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((old_filter, old_parent, old_version)) = previous
            && (version < old_version
                || (version == old_version
                    && (scope.filter != old_filter
                        || scope.parent.map(|id| id.to_string()) != old_parent)))
        {
            return Err(StorageError::PolicyVersionConflict(scope.id));
        }
        tx.execute(
            "INSERT INTO scopes(id, parent_id, name, description, filter, position, policy_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(id) DO UPDATE SET parent_id=excluded.parent_id, name=excluded.name, description=excluded.description, filter=excluded.filter, position=excluded.position, policy_version=excluded.policy_version",
            params![scope.id.to_string(), scope.parent.map(|id| id.to_string()), scope.name, scope.description, scope.filter, scope.position, version],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn load_scopes(&self) -> Result<Vec<Scope>, StorageError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare_cached("SELECT id, parent_id, name, description, filter, position, policy_version FROM scopes ORDER BY position, id")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?;
        rows.map(|row| {
            let (id, parent, name, description, filter, position, policy_version) = row?;
            Ok(Scope {
                id: parse_id(&id)?,
                parent: parent.map(|id| parse_id(&id)).transpose()?,
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
        scope_id: ScopeId,
        version: u64,
        seqs: &[MessageSeq],
    ) -> Result<(), StorageError> {
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let current: Option<i64> = tx
            .query_row(
                "SELECT policy_version FROM scopes WHERE id=?1",
                [scope_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if current.and_then(|value| u64::try_from(value).ok()) != Some(version) {
            return Err(StorageError::StalePolicy(scope_id));
        }
        tx.execute(
            "DELETE FROM message_scope WHERE scope_id=?1",
            [scope_id.to_string()],
        )?;
        {
            let mut statement = tx.prepare_cached("INSERT INTO message_scope(scope_id, message_seq, policy_version) VALUES (?1, ?2, ?3)")?;
            for seq in seqs {
                statement.execute(params![
                    scope_id.to_string(),
                    i64::try_from(seq.0).map_err(|_| StorageError::IntegerRange)?,
                    i64::try_from(version).map_err(|_| StorageError::IntegerRange)?
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}

fn index_message(tx: &Transaction<'_>, seq: i64, message: &Message) -> Result<(), StorageError> {
    let mut recipients = tx.prepare_cached("INSERT OR IGNORE INTO message_recipients(message_seq, address, domain) VALUES (?1, ?2, ?3)")?;
    for mailbox in message
        .facts
        .envelope_to
        .iter()
        .chain(&message.facts.to)
        .chain(&message.facts.cc)
    {
        recipients.execute(params![seq, mailbox.address, mailbox.domain])?;
    }
    drop(recipients);
    let mut headers = tx.prepare_cached(
        "INSERT INTO message_headers(message_seq, name, value, ordinal) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (name, values) in &message.facts.headers {
        for (ordinal, value) in values.iter().enumerate() {
            headers.execute(params![
                seq,
                name,
                value,
                i64::try_from(ordinal).map_err(|_| StorageError::IntegerRange)?
            ])?;
        }
    }
    Ok(())
}

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

fn summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSummary> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
    ))
}

fn decode_summary(
    (seq, id, subject, from, to, received_at, size, attachment_count): StoredSummary,
) -> Result<MessageSummary, StorageError> {
    Ok(MessageSummary {
        seq: MessageSeq(u64::try_from(seq).map_err(|_| StorageError::IntegerRange)?),
        id: parse_id(&id)?,
        subject,
        from: serde_json::from_str(&from)?,
        to: serde_json::from_str(&to)?,
        received_at,
        size: u64::try_from(size).map_err(|_| StorageError::IntegerRange)?,
        attachment_count: u64::try_from(attachment_count)
            .map_err(|_| StorageError::IntegerRange)?,
    })
}

fn parse_id<T: std::str::FromStr>(value: &str) -> Result<T, StorageError> {
    value
        .parse()
        .map_err(|_| StorageError::InvalidData(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandpost_core::{Attachment, Mailbox, MessageFacts};

    fn message() -> Message {
        Message {
            id: MessageId::new(),
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
            raw_mime: b"raw".to_vec(),
            attachments: vec![Attachment {
                filename: Some("a.bin".into()),
                content_type: "application/octet-stream".into(),
                size: 3,
                content_hash: "hash".into(),
            }],
        }
    }
    fn scope(version: u64) -> Scope {
        Scope {
            id: ScopeId::new(),
            parent: None,
            name: "root".into(),
            description: None,
            filter: String::new(),
            position: 0,
            policy_version: version,
        }
    }

    #[test]
    fn migration_is_idempotent_across_reopen_and_schema_must_not_be_newer() {
        let path = std::env::temp_dir().join(format!("sandpost-{}.db", MessageId::new()));
        Storage::open(&path).unwrap();
        Storage::open(&path).unwrap().health().unwrap();
        let db = Connection::open(&path).unwrap();
        db.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        drop(db);
        assert!(matches!(
            Storage::open(&path),
            Err(StorageError::NewerSchema(3))
        ));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn version_one_database_backfills_narrow_summary_columns() {
        let path = std::env::temp_dir().join(format!("sandpost-v1-{}.db", MessageId::new()));
        let original = message();
        let db = Connection::open(&path).unwrap();
        db.execute_batch(include_str!("../migrations/0001_initial.sql"))
            .unwrap();
        db.execute(
            "INSERT INTO messages(id, facts, raw_mime, attachments, sender_domain, received_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                original.id.to_string(),
                serde_json::to_string(&original.facts).unwrap(),
                original.raw_mime,
                serde_json::to_string(&original.attachments).unwrap(),
                "example.org",
                original.facts.received_at,
            ],
        )
        .unwrap();
        db.pragma_update(None, "user_version", 1).unwrap();
        drop(db);

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

    #[test]
    fn ingest_roundtrip_metadata_and_duplicate_rejection() {
        let storage = Storage::memory().unwrap();
        let original = message();
        let seq = storage.insert_message(&original, &[]).unwrap();
        let mut duplicate = original.clone();
        duplicate.raw_mime = b"replacement".to_vec();
        assert!(matches!(
            storage.insert_message(&duplicate, &[]),
            Err(StorageError::DuplicateMessageId(id)) if id == original.id
        ));
        let saved = storage.get_message(original.id).unwrap().unwrap();
        assert_eq!(saved.raw_mime, b"raw");
        assert_eq!(saved.facts, original.facts);
        assert_eq!(saved.attachments, original.attachments);
        assert_eq!(
            storage.get_message_metadata(original.id).unwrap().unwrap(),
            (original.facts.clone(), original.attachments.clone())
        );
        assert_eq!(storage.list_messages(None, 10).unwrap()[0].seq, seq);
    }

    #[test]
    fn foreign_key_failure_rolls_back_message_and_scope_matches() {
        let storage = Storage::memory().unwrap();
        let msg = message();
        let unknown = ScopeId::new();
        assert!(storage.insert_message(&msg, &[(unknown, 1)]).is_err());
        assert!(storage.get_message(msg.id).unwrap().is_none());
    }

    #[test]
    fn pagination_scope_materialization_union_and_policy_guard() {
        let storage = Storage::memory().unwrap();
        let a = scope(3);
        let b = scope(8);
        storage.save_scope(&a).unwrap();
        storage.save_scope(&b).unwrap();
        assert_eq!(storage.load_scopes().unwrap().len(), 2);
        let first = message();
        let first_seq = storage
            .insert_message(&first, &[(a.id, 3), (b.id, 8)])
            .unwrap();
        let mut second = message();
        second.id = MessageId::new();
        let second_seq = storage.insert_message(&second, &[(a.id, 3)]).unwrap();
        let db = storage.connection().unwrap();
        let count: i64 = db
            .query_row("SELECT count(*) FROM message_scope", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 3);
        drop(db);
        assert_eq!(storage.list_messages(None, 1).unwrap()[0].seq, second_seq);
        assert_eq!(
            storage.list_messages(Some(second_seq), 10).unwrap()[0].seq,
            first_seq
        );
        let visible = storage.list_visible_messages(&[a.id], None, 10).unwrap();
        assert_eq!(
            visible.iter().map(|row| row.seq).collect::<Vec<_>>(),
            vec![second_seq, first_seq]
        );
        assert!(
            matches!(storage.replace_scope_matches(a.id, 2, &[first_seq]), Err(StorageError::StalePolicy(id)) if id == a.id)
        );
        storage
            .replace_scope_matches(a.id, 3, &[first_seq])
            .unwrap();
        let db = storage.connection().unwrap();
        let rows: i64 = db
            .query_row(
                "SELECT count(*) FROM message_scope WHERE scope_id=?1 AND policy_version=3",
                [a.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1);
        drop(db);
        let mut changed = a.clone();
        changed.filter = "subject contains 'changed'".into();
        assert!(
            matches!(storage.save_scope(&changed), Err(StorageError::PolicyVersionConflict(id)) if id == a.id)
        );
        changed.policy_version = 4;
        storage.save_scope(&changed).unwrap();
        assert!(
            storage
                .list_visible_messages(&[a.id], None, 10)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            storage
                .list_visible_messages(&[a.id, b.id], None, 10)
                .unwrap()
                .len(),
            1
        );
        let mut reparented = changed.clone();
        reparented.parent = Some(b.id);
        assert!(matches!(
            storage.save_scope(&reparented),
            Err(StorageError::PolicyVersionConflict(id)) if id == a.id
        ));
        storage
            .replace_scope_matches(a.id, 4, &[first_seq])
            .unwrap();
        assert_eq!(
            storage
                .list_visible_messages(&[a.id], None, 10)
                .unwrap()
                .len(),
            1
        );
    }
}
