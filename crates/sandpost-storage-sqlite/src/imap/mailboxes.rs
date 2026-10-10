//! IMAP mailbox identity reconciliation and materialized membership synchronization.

use super::accounts::get_account_blocking;
use super::shared::{
    MAILBOX_COLUMNS, MAXIMUM_UID, allocate_uid_validity, decode_mailbox, mailbox_state,
    source_columns,
};
use crate::error::StorageResult;
use crate::filter::compile;
use rusqlite::{Connection, params, params_from_iter, types::Value as SqlValue};
use sandpost_core::{ImapAccountIdentifier, ImapMailboxIdentifier, ImapMailboxSource};
use sandpost_storage::{ImapMailboxState, ImapMembershipDefinition, StorageError};
use std::collections::HashMap;

/// Create missing mailbox identities and delete identities whose source is no longer listed.
///
/// The transaction is deferred, so reconciling an unchanged source list only reads. Returned
/// states preserve the source order supplied by the caller.
pub(super) fn reconcile_mailboxes_blocking(
    connection: &mut Connection,
    account: ImapAccountIdentifier,
    sources: &[ImapMailboxSource],
) -> Result<Vec<ImapMailboxState>, StorageError> {
    let transaction = connection.transaction().storage()?;
    if get_account_blocking(&transaction, account)?.is_none() {
        return Err(StorageError::NotFound);
    }
    let existing: Vec<ImapMailboxState> = {
        let mut statement = transaction
            .prepare(&format!(
                "SELECT {MAILBOX_COLUMNS} FROM imap_mailboxes WHERE account_identifier = ?1"
            ))
            .storage()?;
        statement
            .query_map([account.to_string()], decode_mailbox)
            .storage()?
            .collect::<Result<_, _>>()
            .storage()?
    };
    let mut by_source: HashMap<ImapMailboxSource, ImapMailboxState> = HashMap::new();
    for state in existing {
        if sources.contains(&state.source) {
            by_source.insert(state.source, state);
        } else {
            transaction
                .execute(
                    "DELETE FROM imap_mailboxes WHERE identifier = ?1",
                    [state.identifier.to_string()],
                )
                .storage()?;
        }
    }
    let mut states = Vec::with_capacity(sources.len());
    for source in sources {
        if let Some(state) = by_source.get(source) {
            states.push(*state);
            continue;
        }
        let identifier = ImapMailboxIdentifier::new();
        let uid_validity = allocate_uid_validity(&transaction)?;
        let (kind, view, folder) = source_columns(*source);
        transaction
            .execute(
                "INSERT INTO imap_mailboxes(identifier,account_identifier,source,view_identifier,folder_identifier,uid_validity) VALUES (?1,?2,?3,?4,?5,?6)",
                params![identifier.to_string(), account.to_string(), kind, view, folder, uid_validity],
            )
            .map_err(|error| match crate::error::sqlite_error(error) {
                StorageError::ConstraintViolation(_) => StorageError::NotFound,
                error => error,
            })?;
        let state = ImapMailboxState {
            identifier,
            account_identifier: account,
            source: *source,
            uid_validity,
            uid_next: 1,
            recent_through: 0,
        };
        by_source.insert(*source, state);
        states.push(state);
    }
    transaction.commit().storage()?;
    Ok(states)
}

/// transaction only reads.
pub(super) fn synchronize_mailbox_blocking(
    connection: &mut Connection,
    identifier: ImapMailboxIdentifier,
    definition: &ImapMembershipDefinition,
) -> Result<Option<ImapMailboxState>, StorageError> {
    let transaction = connection.transaction().storage()?;
    let Some(state) = mailbox_state(&transaction, identifier)? else {
        return Ok(None);
    };
    let (examined_sequence, stored_fingerprint, uid_next): (i64, Option<String>, i64) = transaction
        .query_row(
            "SELECT examined_sequence, definition_fingerprint, uid_next FROM imap_mailboxes WHERE identifier = ?1",
            [identifier.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .storage()?;
    let latest_sequence: i64 = transaction
        .query_row("SELECT coalesce(max(sequence), 0) FROM mail", [], |row| {
            row.get(0)
        })
        .storage()?;
    let full = stored_fingerprint.as_deref() != Some(definition.fingerprint.as_str());
    if !full && latest_sequence == examined_sequence {
        transaction.commit().storage()?;
        return Ok(Some(state));
    }
    let compiled = compile(&definition.filter);
    if full {
        let mut parameters = vec![SqlValue::Text(identifier.to_string())];
        parameters.extend(compiled.parameters.iter().cloned());
        transaction
            .execute(
                &format!(
                    "DELETE FROM imap_mailbox_messages WHERE mailbox_identifier = ? AND mail_sequence NOT IN (SELECT mail.sequence FROM mail WHERE ({}))",
                    compiled.sql
                ),
                params_from_iter(parameters),
            )
            .storage()?;
    }
    let mut next_uid = uid_next;
    if !state.source.is_explicit() {
        let start = if full { 0 } else { examined_sequence };
        if latest_sequence > start {
            let mut parameters = vec![
                SqlValue::Text(identifier.to_string()),
                SqlValue::Integer(uid_next),
                SqlValue::Integer(start),
                SqlValue::Integer(latest_sequence),
            ];
            parameters.extend(compiled.parameters.iter().cloned());
            parameters.push(SqlValue::Text(identifier.to_string()));
            parameters.push(SqlValue::Text(identifier.to_string()));
            let inserted = transaction
                .execute(
                    &format!(
                        "INSERT INTO imap_mailbox_messages(mailbox_identifier, uid, mail_sequence) \
                         SELECT ?, ? + ROW_NUMBER() OVER (ORDER BY mail.sequence) - 1, mail.sequence FROM mail \
                         WHERE mail.sequence > ? AND mail.sequence <= ? AND ({}) \
                         AND NOT EXISTS (SELECT 1 FROM imap_mailbox_messages AS member WHERE member.mailbox_identifier = ? AND member.mail_sequence = mail.sequence) \
                         AND NOT EXISTS (SELECT 1 FROM imap_mailbox_exclusions AS exclusion WHERE exclusion.mailbox_identifier = ? AND exclusion.mail_sequence = mail.sequence) \
                         ORDER BY mail.sequence",
                        compiled.sql
                    ),
                    params_from_iter(parameters),
                )
                .storage()?;
            next_uid += i64::try_from(inserted).map_err(|_| StorageError::IntegerRange)?;
            if next_uid > MAXIMUM_UID + 1 {
                return Err(StorageError::IntegerRange);
            }
        }
    }
    transaction
        .execute(
            "UPDATE imap_mailboxes SET uid_next = ?1, examined_sequence = ?2, definition_fingerprint = ?3 WHERE identifier = ?4",
            params![
                next_uid,
                latest_sequence,
                definition.fingerprint,
                identifier.to_string()
            ],
        )
        .storage()?;
    let updated = mailbox_state(&transaction, identifier)?;
    transaction.commit().storage()?;
    Ok(updated)
}

/// Load one mailbox identity by identifier.
pub(super) fn get_mailbox_blocking(
    connection: &Connection,
    identifier: ImapMailboxIdentifier,
) -> Result<Option<ImapMailboxState>, StorageError> {
    mailbox_state(connection, identifier)
}
