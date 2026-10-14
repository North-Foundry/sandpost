//! IMAP membership mutations: copy, expunge, and exclusion management.

use super::shared::{MAXIMUM_UID, mailbox_state, uid_from};
use crate::error::StorageResult;
use crate::filter::{bind_query, compile};
use sandpost_core::{ImapAccountIdentifier, ImapMailboxIdentifier};
use sandpost_storage::{ImapCopyResult, ImapMembershipDefinition, StorageError};
use sqlx::{Row, SqliteConnection};
use std::collections::{HashMap, HashSet};

/// Keep UID filters below SQLite's supported bound-parameter limit.
const UID_BATCH_SIZE: usize = 500;

/// Expunge flagged members of one mailbox and return their UIDs.
///
/// Clients without MOVE transfer mail with COPY, `\Deleted`, and EXPUNGE. When the expunged
/// occurrence is the account's primary one and an explicit COPY occurrence survives elsewhere in
/// view, that copy becomes its primary placement. Other occurrences are removed locally,
/// with the final normal occurrence entering Trash and final Trash releasing the view.
pub(super) async fn expunge(
    connection: &mut SqliteConnection,
    identifier: ImapMailboxIdentifier,
    uids: Option<&[u32]>,
) -> Result<Vec<u32>, StorageError> {
    let Some(state) = mailbox_state(connection, identifier).await? else {
        return Ok(Vec::new());
    };
    let organization =
        super::organization::organization_for_mailbox(connection, identifier).await?;
    let source_target = organization.target(connection, state.source).await?;
    let mailbox_id = identifier.to_string();
    let mut flagged_members = Vec::new();
    match uids {
        Some(uids) => {
            for batch in uids.chunks(UID_BATCH_SIZE) {
                if batch.is_empty() {
                    continue;
                }
                let placeholders = vec!["?"; batch.len()].join(",");
                let sql = format!(
                    "SELECT uid AS member_uid, mail_sequence FROM imap_mailbox_messages WHERE mailbox_identifier = ? AND deleted = 1 AND uid IN ({placeholders}) ORDER BY uid"
                );
                let mut query = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(&mailbox_id);
                for uid in batch {
                    query = query.bind(uid);
                }
                flagged_members.extend(query.fetch_all(&mut *connection).await.storage()?);
            }
        }
        None => {
            flagged_members = sqlx::query("SELECT uid AS member_uid, mail_sequence FROM imap_mailbox_messages WHERE mailbox_identifier = ? AND deleted = 1 ORDER BY uid")
                .bind(&mailbox_id)
                .fetch_all(&mut *connection)
                .await
                .storage()?;
        }
    }
    let mut members = flagged_members
        .iter()
        .map(|row| {
            Ok((
                row.try_get::<i64, _>("member_uid").storage()?,
                row.try_get::<i64, _>("mail_sequence").storage()?,
            ))
        })
        .collect::<Result<Vec<_>, StorageError>>()?;
    members.sort_unstable();
    members.dedup();
    for (uid, sequence) in &members {
        organization
            .remove(connection, *sequence, &source_target)
            .await?;
        sqlx::query("DELETE FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND uid = ?2")
            .bind(&mailbox_id)
            .bind(uid)
            .execute(&mut *connection)
            .await
            .storage()?;
    }
    if !members.is_empty() {
        organization.invalidate(connection).await?;
        super::organization::collect_unreferenced_messages(connection).await?;
    }
    members.iter().map(|(uid, _)| uid_from(*uid)).collect()
}

/// Copy or move members between two mailboxes of one account.
pub(super) async fn copy(
    connection: &mut SqliteConnection,
    source: ImapMailboxIdentifier,
    uids: &[u32],
    destination: ImapMailboxIdentifier,
    destination_definition: Option<&ImapMembershipDefinition>,
    remove_from_source: bool,
) -> Result<ImapCopyResult, StorageError> {
    let source_state = mailbox_state(connection, source)
        .await?
        .ok_or(StorageError::NotFound)?;
    let destination_state = mailbox_state(connection, destination)
        .await?
        .ok_or(StorageError::NotFound)?;
    if source_state.account_identifier != destination_state.account_identifier {
        return Err(StorageError::NotFound);
    }
    let members = member_sequences(connection, source, uids).await?;
    if !members.is_empty() && destination_definition.is_none() {
        return Err(StorageError::ConstraintViolation(
            "a destination requires its authorization definition".into(),
        ));
    }
    if matches!(
        destination_state.source,
        sandpost_core::ImapMailboxSource::View(_)
    ) {
        return Err(StorageError::ConstraintViolation(
            "messages cannot be inserted into a smart View".into(),
        ));
    }
    if let sandpost_core::ImapMailboxSource::LocalFolder(folder) = destination_state.source {
        let kind: Option<String> = sqlx::query_scalar(
            "SELECT kind FROM imap_local_folders WHERE identifier = ?1 AND account_identifier = ?2",
        )
        .bind(folder.to_string())
        .bind(destination_state.account_identifier.to_string())
        .fetch_optional(&mut *connection)
        .await
        .storage()?;
        if kind.as_deref() != Some("manual") && kind.as_deref() != Some("private") {
            return Err(StorageError::ConstraintViolation(
                "messages cannot be inserted into a smart folder".into(),
            ));
        }
    }
    if let Some(definition) = destination_definition {
        let compiled = compile(&definition.filter);
        let sql = format!(
            "SELECT EXISTS(SELECT 1 FROM mail WHERE mail.sequence = ? AND ({})) AS matches_definition",
            compiled.sql
        );
        for (_, sequence) in &members {
            let row = bind_query(
                sqlx::query(sqlx::AssertSqlSafe(sql.as_str())).bind(*sequence),
                &compiled.parameters,
            )
            .fetch_one(&mut *connection)
            .await
            .storage()?;
            let matches: bool = row.try_get("matches_definition").storage()?;
            if !matches {
                return Err(StorageError::ConstraintViolation(
                    "a message does not match the destination mailbox".into(),
                ));
            }
        }
    }
    let mut next_uid: i64 =
        sqlx::query_scalar("SELECT uid_next FROM imap_mailboxes WHERE identifier = ?1")
            .bind(destination.to_string())
            .fetch_one(&mut *connection)
            .await
            .storage()?;
    let mut result = ImapCopyResult {
        uid_validity: destination_state.uid_validity,
        all_new: true,
        ..ImapCopyResult::default()
    };
    let organization = super::organization::organization_for_mailbox(connection, source).await?;
    let destination_organization =
        super::organization::organization_for_mailbox(connection, destination).await?;
    if organization.mirror != destination_organization.mirror {
        return Err(StorageError::ConstraintViolation(
            "cannot transfer between organization modes".into(),
        ));
    }
    let source_target = if remove_from_source {
        organization.target(connection, source_state.source).await?
    } else {
        super::organization::Target::fixed("smart")
    };
    let destination_target = organization
        .target(connection, destination_state.source)
        .await?;
    // Apple Mail deletes through COPY to Trash followed by STORE/EXPUNGE. A retained Trash
    // copy must not reject an entire selection; coalesce only cross-mailbox Trash transfers.
    let coalesce_trash = source != destination && destination_target.kind == "trash";
    for (uid, sequence) in &members {
        // Another Mirror credential may have committed the destination occurrence after its
        // projection was synchronized. Canonical organization is authoritative inside this write.
        let primary = organization.primary(connection, *sequence).await?;
        let destination_present = primary == destination_target
            || organization
                .copies(connection, *sequence)
                .await?
                .contains(&destination_target);
        if !coalesce_trash && destination_present {
            return Err(StorageError::ConstraintViolation(
                "destination already contains a message".into(),
            ));
        }
        let copied_occurrence = remove_from_source && primary != source_target;
        sqlx::query("DELETE FROM imap_mailbox_exclusions WHERE mailbox_identifier = ?1 AND mail_sequence = ?2")
            .bind(destination.to_string()).bind(sequence).execute(&mut *connection).await.storage()?;
        let existing: Option<i64> = sqlx::query_scalar("SELECT uid FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND mail_sequence = ?2")
            .bind(destination.to_string()).bind(sequence).fetch_optional(&mut *connection).await.storage()?;
        let destination_uid = match existing {
            Some(existing) if coalesce_trash && destination_present => {
                result.all_new = false;
                existing
            }
            Some(_) => {
                return Err(StorageError::ConstraintViolation(
                    "destination already contains a message".into(),
                ));
            }
            None => {
                if next_uid > MAXIMUM_UID {
                    return Err(StorageError::IntegerRange);
                }
                sqlx::query("INSERT INTO imap_mailbox_messages(mailbox_identifier, uid, mail_sequence, deleted) SELECT ?1, ?2, mail_sequence, deleted FROM imap_mailbox_messages WHERE mailbox_identifier = ?3 AND uid = ?4")
                    .bind(destination.to_string()).bind(next_uid).bind(source.to_string()).bind(uid)
                    .execute(&mut *connection).await.storage()?;
                next_uid += 1;
                next_uid - 1
            }
        };
        result.source_uids.push(*uid);
        result.destination_uids.push(uid_from(destination_uid)?);
        if remove_from_source && !copied_occurrence {
            organization
                .copy_record(connection, *sequence, &destination_target, false)
                .await?;
            organization
                .place(connection, *sequence, &destination_target)
                .await?;
        } else if primary != destination_target {
            organization
                .copy_record(connection, *sequence, &destination_target, true)
                .await?;
        }
    }
    sqlx::query("UPDATE imap_mailboxes SET uid_next = ?1 WHERE identifier = ?2")
        .bind(next_uid)
        .bind(destination.to_string())
        .execute(&mut *connection)
        .await
        .storage()?;
    if remove_from_source && source != destination {
        for (uid, sequence) in &members {
            // A MOVE removes only the source occurrence, preserving unrelated explicit copies.
            organization
                .copy_record(connection, *sequence, &source_target, false)
                .await?;
            sqlx::query(
                "DELETE FROM imap_mailbox_messages WHERE mailbox_identifier = ?1 AND uid = ?2",
            )
            .bind(source.to_string())
            .bind(uid)
            .execute(&mut *connection)
            .await
            .storage()?;
            result.removed_uids.push(*uid);
        }
    }
    if !members.is_empty() {
        organization.invalidate(connection).await?;
    }
    Ok(result)
}

/// Clear legacy account exclusions and reconcile, preserving placements and user deletion tombstones.
pub(super) async fn clear_exclusions(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
) -> Result<u64, StorageError> {
    let cleared = sqlx::query("DELETE FROM imap_mailbox_exclusions WHERE mailbox_identifier IN (SELECT identifier FROM imap_mailboxes WHERE account_identifier = ?1)")
        .bind(account.to_string()).execute(&mut *connection).await.storage()?.rows_affected();
    sqlx::query(
        "UPDATE imap_mailboxes SET definition_fingerprint = NULL WHERE account_identifier = ?1",
    )
    .bind(account.to_string())
    .execute(connection)
    .await
    .storage()?;
    Ok(cleared)
}

/// Load each non-deleted owner's message sequence for the supplied member UIDs.
pub(super) async fn member_sequences(
    connection: &mut SqliteConnection,
    mailbox: ImapMailboxIdentifier,
    uids: &[u32],
) -> Result<Vec<(u32, i64)>, StorageError> {
    let organization = super::organization::organization_for_mailbox(connection, mailbox).await?;
    let mut found = HashMap::with_capacity(uids.len());
    for batch in uids.chunks(UID_BATCH_SIZE) {
        if batch.is_empty() {
            continue;
        }
        let placeholders = vec!["?"; batch.len()].join(",");
        let sql = format!(
            "SELECT member.uid, member.mail_sequence FROM imap_mailbox_messages AS member WHERE member.mailbox_identifier = ? AND member.uid IN ({placeholders}) AND ((? = 1 AND NOT EXISTS (SELECT 1 FROM user_message_deletions AS deletion WHERE deletion.user_identifier = ? AND deletion.mail_sequence = member.mail_sequence)) OR (? = 0 AND EXISTS(SELECT 1 FROM imap_account_messages WHERE account_identifier = ? AND mail_sequence = member.mail_sequence)))"
        );
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(mailbox.to_string());
        for uid in batch {
            query = query.bind(uid);
        }
        let rows = query
            .bind(organization.mirror)
            .bind(&organization.identifier)
            .bind(organization.mirror)
            .bind(&organization.account)
            .fetch_all(&mut *connection)
            .await
            .storage()?;
        for row in &rows {
            found.insert(
                uid_from(row.try_get("uid").storage()?)?,
                row.try_get("mail_sequence").storage()?,
            );
        }
    }
    let mut unique = HashSet::new();
    Ok(uids
        .iter()
        .filter(|uid| unique.insert(**uid))
        .filter_map(|uid| found.get(uid).map(|sequence| (*uid, *sequence)))
        .collect())
}
