//! IMAP membership mutations: copy, expunge, and exclusion management.

use super::shared::{MAXIMUM_UID, mailbox_state, uid_from};
use crate::error::StorageResult;
use crate::filter::compile;
use rusqlite::{
    Connection, OptionalExtension, Transaction, TransactionBehavior, params, params_from_iter,
    types::Value as SqlValue,
};
use sandpost_core::{ImapAccountIdentifier, ImapMailboxIdentifier};
use sandpost_storage::{ImapCopyResult, ImapMembershipDefinition, StorageError};

/// Remove members flagged deleted and remember expunges from dynamic mailboxes.
pub(super) fn expunge_blocking(
    connection: &mut Connection,
    identifier: ImapMailboxIdentifier,
    uids: Option<&[u32]>,
) -> Result<Vec<u32>, StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    let Some(state) = mailbox_state(&transaction, identifier)? else {
        return Ok(Vec::new());
    };
    let candidates: Vec<(i64, i64)> = {
        let mut statement = transaction
            .prepare(
                "SELECT uid, mail_sequence FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND deleted = 1 ORDER BY uid",
            )
            .storage()?;
        statement
            .query_map([identifier.to_string()], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .storage()?
            .collect::<Result<_, _>>()
            .storage()?
    };
    let mut removed = Vec::new();
    for (uid, sequence) in candidates {
        let uid = uid_from(uid)?;
        if uids.is_some_and(|uids| !uids.contains(&uid)) {
            continue;
        }
        transaction
            .execute(
                "DELETE FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND uid = ?2",
                params![identifier.to_string(), uid],
            )
            .storage()?;
        if !state.source.is_explicit() {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO imap_mailbox_exclusions(mailbox_identifier, mail_sequence) VALUES (?1, ?2)",
                    params![identifier.to_string(), sequence],
                )
                .storage()?;
        }
        removed.push(uid);
    }
    transaction.commit().storage()?;
    Ok(removed)
}

/// Copy or move members between two mailboxes of one account.
pub(super) fn copy_blocking(
    connection: &mut Connection,
    source: ImapMailboxIdentifier,
    uids: &[u32],
    destination: ImapMailboxIdentifier,
    destination_definition: Option<&ImapMembershipDefinition>,
    remove_from_source: bool,
) -> Result<ImapCopyResult, StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    let source_state = mailbox_state(&transaction, source)?.ok_or(StorageError::NotFound)?;
    let destination_state =
        mailbox_state(&transaction, destination)?.ok_or(StorageError::NotFound)?;
    if source_state.account_identifier != destination_state.account_identifier {
        return Err(StorageError::NotFound);
    }
    let members = member_sequences(&transaction, source, uids)?;
    if let Some(definition) = destination_definition {
        let compiled = compile(&definition.filter);
        let sql = format!(
            "SELECT EXISTS(SELECT 1 FROM mail WHERE mail.sequence = ? AND ({}))",
            compiled.sql
        );
        for (_, sequence) in &members {
            let mut parameters = vec![SqlValue::Integer(*sequence)];
            parameters.extend(compiled.parameters.iter().cloned());
            let matches: bool = transaction
                .query_row(&sql, params_from_iter(parameters), |row| row.get(0))
                .storage()?;
            if !matches {
                return Err(StorageError::ConstraintViolation(
                    "a message does not match the destination mailbox".into(),
                ));
            }
        }
    }
    let mut next_uid = i64::from(destination_state.uid_next);
    let mut result = ImapCopyResult {
        uid_validity: destination_state.uid_validity,
        all_new: true,
        ..ImapCopyResult::default()
    };
    for (uid, sequence) in &members {
        transaction
            .execute(
                "DELETE FROM imap_mailbox_exclusions WHERE mailbox_identifier = ?1 AND mail_sequence = ?2",
                params![destination.to_string(), sequence],
            )
            .storage()?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT uid FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND mail_sequence = ?2",
                params![destination.to_string(), sequence],
                |row| row.get(0),
            )
            .optional()
            .storage()?;
        let destination_uid = match existing {
            Some(existing) => {
                result.all_new = false;
                existing
            }
            None => {
                if next_uid > MAXIMUM_UID {
                    return Err(StorageError::IntegerRange);
                }
                transaction
                    .execute(
                        "INSERT INTO imap_mailbox_messages(mailbox_identifier, uid, mail_sequence) VALUES (?1, ?2, ?3)",
                        params![destination.to_string(), next_uid, sequence],
                    )
                    .storage()?;
                next_uid += 1;
                next_uid - 1
            }
        };
        result.source_uids.push(*uid);
        result.destination_uids.push(uid_from(destination_uid)?);
    }
    transaction
        .execute(
            "UPDATE imap_mailboxes SET uid_next = ?1 WHERE identifier = ?2",
            params![next_uid, destination.to_string()],
        )
        .storage()?;
    if remove_from_source && source != destination {
        for (uid, sequence) in &members {
            transaction
                .execute(
                    "DELETE FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND uid = ?2",
                    params![source.to_string(), uid],
                )
                .storage()?;
            if !source_state.source.is_explicit() {
                transaction
                    .execute(
                        "INSERT OR IGNORE INTO imap_mailbox_exclusions(mailbox_identifier, mail_sequence) VALUES (?1, ?2)",
                        params![source.to_string(), sequence],
                    )
                    .storage()?;
            }
            result.removed_uids.push(*uid);
        }
    }
    transaction.commit().storage()?;
    Ok(result)
}

/// Forget an account's expunges and force full reconciliation of its mailboxes.
pub(super) fn clear_exclusions_blocking(
    connection: &mut Connection,
    account: ImapAccountIdentifier,
) -> Result<u64, StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    let cleared = transaction
        .execute(
            "DELETE FROM imap_mailbox_exclusions WHERE mailbox_identifier IN (SELECT identifier FROM imap_mailboxes WHERE account_identifier = ?1)",
            [account.to_string()],
        )
        .storage()?;
    transaction
        .execute(
            "UPDATE imap_mailboxes SET definition_fingerprint = NULL WHERE account_identifier = ?1",
            [account.to_string()],
        )
        .storage()?;
    transaction.commit().storage()?;
    Ok(cleared as u64)
}

/// Load the message sequence of each existing member UID, in the given UID order.
pub(super) fn member_sequences(
    transaction: &Transaction<'_>,
    mailbox: ImapMailboxIdentifier,
    uids: &[u32],
) -> Result<Vec<(u32, i64)>, StorageError> {
    let mut statement = transaction
        .prepare_cached(
            "SELECT mail_sequence FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND uid = ?2",
        )
        .storage()?;
    let mut members = Vec::with_capacity(uids.len());
    for uid in uids {
        if let Some(sequence) = statement
            .query_row(params![mailbox.to_string(), uid], |row| {
                row.get::<_, i64>(0)
            })
            .optional()
            .storage()?
        {
            members.push((*uid, sequence));
        }
    }
    Ok(members)
}
