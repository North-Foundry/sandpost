//! Versioned scope materializations and deduplicated visibility reads.
use crate::{
    MessageSummary, Storage, StorageError,
    messages::MAXIMUM_PAGE_SIZE,
    records::{decode_summaries, summary_row},
};
use rusqlite::{OptionalExtension, params};
use sandpost_core::{MessageSequence, ScopeIdentifier};

impl Storage {
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
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        let limit = limit.min(MAXIMUM_PAGE_SIZE);
        let scope_placeholders = (2..scope_identifiers.len() + 2)
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let query_statement = format!(
            "WITH visible AS MATERIALIZED (\
                SELECT scope_matches.mail_sequence FROM mail_scope scope_matches \
                JOIN scopes current_scopes ON current_scopes.identifier = scope_matches.scope_identifier AND scope_matches.policy_version = current_scopes.policy_version \
                WHERE scope_matches.scope_identifier IN ({scope_placeholders}) AND (?1 IS NULL OR scope_matches.mail_sequence < ?1) \
                GROUP BY scope_matches.mail_sequence ORDER BY scope_matches.mail_sequence DESC LIMIT ?{}\
             ) SELECT stored_mail.sequence, stored_mail.identifier, stored_mail.subject, stored_mail.received_at, stored_mail.size, \
                (SELECT COUNT(*) FROM mail_attachments WHERE mail_sequence = stored_mail.sequence) \
             FROM visible JOIN mail stored_mail ON stored_mail.sequence = visible.mail_sequence ORDER BY stored_mail.sequence DESC",
            scope_identifiers.len() + 2,
        );
        let mut statement = transaction.prepare_cached(&query_statement)?;
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
        let rows = statement
            .query_map(rusqlite::params_from_iter(values), summary_row)?
            .collect::<Result<Vec<_>, _>>()?;
        decode_summaries(&transaction, rows)
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
                "SELECT policy_version FROM scopes WHERE identifier=?1",
                [scope_identifier.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if current.and_then(|value| u64::try_from(value).ok()) != Some(policy_version) {
            return Err(StorageError::StalePolicy(scope_identifier));
        }
        transaction.execute(
            "DELETE FROM mail_scope WHERE scope_identifier=?1",
            [scope_identifier.to_string()],
        )?;
        {
            let mut statement = transaction.prepare_cached("INSERT INTO mail_scope(scope_identifier, mail_sequence, policy_version) VALUES (?1, ?2, ?3)")?;
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
