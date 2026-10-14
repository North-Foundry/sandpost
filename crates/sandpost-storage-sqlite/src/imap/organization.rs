//! Shared primary and durable independent organization without duplicating message content.
use crate::error::StorageResult;
use sandpost_core::{
    ImapAccountIdentifier, ImapFolderIdentifier, ImapMailboxIdentifier, ImapMailboxSource,
};
use sandpost_storage::StorageError;
use sqlx::{Row, SqliteConnection};

/// An occurrence's stable organization target, independent of mailbox UIDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub(super) kind: String,
    pub(super) folder: Option<String>,
}

impl Target {
    /// Construct a fixed Inbox or Trash destination.
    pub(super) fn fixed(kind: &str) -> Self {
        Self {
            kind: kind.into(),
            folder: None,
        }
    }
}

/// The state owner of a mailbox; Mirror state belongs to the primary user.
pub(super) struct Organization {
    pub(super) mirror: bool,
    pub(super) identifier: String,
    pub(super) account: String,
}

/// Require an enabled account's captured generation before a fenced transaction mutates it.
pub(super) async fn require_account_generation(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
    expected: Option<u64>,
) -> Result<(), StorageError> {
    let current = sqlx::query_as::<_, (i64, bool, bool)>(
        "SELECT mode_generation, enabled, 1 FROM imap_accounts WHERE identifier=?1",
    )
    .bind(account.to_string())
    .fetch_optional(connection)
    .await
    .storage()?;
    require_generation(current, expected)
}

/// Fence a mailbox's generation and reject dormant-mode handles, including unchanged membership.
pub(super) async fn require_mailbox_generation(
    connection: &mut SqliteConnection,
    mailbox: ImapMailboxIdentifier,
    expected: Option<u64>,
) -> Result<(), StorageError> {
    let current = sqlx::query_as::<_, (i64, bool, bool)>(
        "SELECT account.mode_generation, account.enabled, box.organization_mode=account.mode FROM imap_mailboxes AS box JOIN imap_accounts AS account ON account.identifier=box.account_identifier WHERE box.identifier=?1",
    )
    .bind(mailbox.to_string())
    .fetch_optional(connection)
    .await
    .storage()?;
    require_generation(current, expected)
}

/// Fence owner configuration without requiring the credential to accept IMAP sessions.
pub(super) async fn require_account_configuration_generation(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
    expected: Option<u64>,
) -> Result<(), StorageError> {
    let current = super::accounts::get_account(connection, account)
        .await?
        .ok_or(StorageError::NotFound)?;
    if expected.is_some_and(|expected| expected != current.mode_generation) {
        return Err(StorageError::NotFound);
    }
    Ok(())
}

/// Fence active folder configuration, allowing its owner to manage disabled credentials.
pub(super) async fn require_folder_configuration_generation(
    connection: &mut SqliteConnection,
    folder: ImapFolderIdentifier,
    expected: Option<u64>,
) -> Result<(), StorageError> {
    let current = sqlx::query_as::<_, (String, bool)>(
        "SELECT account.identifier, folder.organization_mode=account.mode FROM imap_local_folders AS folder JOIN imap_accounts AS account ON account.identifier=folder.account_identifier WHERE folder.identifier=?1",
    )
    .bind(folder.to_string())
    .fetch_optional(&mut *connection)
    .await
    .storage()?;
    let Some((account, active)) = current else {
        return if expected.is_none() {
            Ok(())
        } else {
            Err(StorageError::NotFound)
        };
    };
    if !active {
        return Err(StorageError::NotFound);
    }
    require_account_configuration_generation(
        connection,
        crate::records::parse_identifier(&account)?,
        expected,
    )
    .await
}

/// Preserve unfenced missing-record semantics while rejecting inactive handles and stale sessions.
fn require_generation(
    current: Option<(i64, bool, bool)>,
    expected: Option<u64>,
) -> Result<(), StorageError> {
    let Some((generation, enabled, active)) = current else {
        return if expected.is_none() {
            Ok(())
        } else {
            Err(StorageError::NotFound)
        };
    };
    let generation = u64::try_from(generation).map_err(|_| StorageError::IntegerRange)?;
    if !active || expected.is_some_and(|expected| !enabled || expected != generation) {
        return Err(StorageError::NotFound);
    }
    Ok(())
}

impl Organization {
    /// SQL selecting currently retained messages and the requested occurrence or dynamic smart result.
    pub(super) fn membership_policy(&self, source: ImapMailboxSource) -> String {
        if self.mirror {
            let owner = "(SELECT owner_identifier FROM imap_accounts WHERE identifier = box.account_identifier)";
            let retained = format!(
                "NOT EXISTS(SELECT 1 FROM user_message_deletions WHERE user_identifier = {owner} AND mail_sequence = mail.sequence)"
            );
            let trashed = format!(
                "EXISTS(SELECT 1 FROM user_message_trash WHERE owner_identifier = {owner} AND mail_sequence = mail.sequence)"
            );
            let filed = format!(
                "EXISTS(SELECT 1 FROM user_message_placements WHERE owner_identifier = {owner} AND mail_sequence = mail.sequence AND folder_identifier IS NOT NULL)"
            );
            let smart_trashed = format!(
                "({trashed}) OR EXISTS(SELECT 1 FROM user_message_copies WHERE owner_identifier = {owner} AND mail_sequence = mail.sequence AND target_kind = 'trash')"
            );
            let primary = match source {
                ImapMailboxSource::Inbox => format!("NOT ({trashed}) AND NOT ({filed})"),
                ImapMailboxSource::Trash => trashed.clone(),
                ImapMailboxSource::LocalFolder(_) => format!(
                    "NOT ({trashed}) AND EXISTS(SELECT 1 FROM user_message_placements WHERE owner_identifier = {owner} AND mail_sequence = mail.sequence AND folder_identifier = local.definition_identifier)"
                ),
                ImapMailboxSource::View(_) => format!("NOT ({smart_trashed})"),
            };
            let copy = format!(
                "EXISTS(SELECT 1 FROM user_message_copies WHERE owner_identifier = {owner} AND mail_sequence = mail.sequence AND target_kind = box.source AND folder_identifier IS local.definition_identifier)"
            );
            let primary = if matches!(source, ImapMailboxSource::LocalFolder(_)) {
                format!(
                    "(local.kind = 'smart' AND NOT ({smart_trashed})) OR (local.kind != 'smart' AND ({primary}))"
                )
            } else {
                primary
            };
            let smart = matches!(source, ImapMailboxSource::View(_));
            format!(
                "({retained}) AND ({primary} OR ({}))",
                if smart { "0" } else { &copy }
            )
        } else {
            let retained = "EXISTS(SELECT 1 FROM imap_account_messages WHERE account_identifier = box.account_identifier AND mail_sequence = mail.sequence)";
            let placed = "EXISTS(SELECT 1 FROM imap_message_placements WHERE account_identifier = box.account_identifier AND mail_sequence = mail.sequence)";
            let primary = match source {
                ImapMailboxSource::Inbox => format!("NOT ({placed}) OR EXISTS(SELECT 1 FROM imap_message_placements WHERE account_identifier = box.account_identifier AND mail_sequence = mail.sequence AND target_kind = 'inbox')"),
                ImapMailboxSource::Trash => "EXISTS(SELECT 1 FROM imap_message_placements WHERE account_identifier = box.account_identifier AND mail_sequence = mail.sequence AND target_kind = 'trash')".into(),
                ImapMailboxSource::LocalFolder(_) => "EXISTS(SELECT 1 FROM imap_message_placements WHERE account_identifier = box.account_identifier AND mail_sequence = mail.sequence AND target_kind = 'folder' AND folder_identifier = box.folder_identifier)".into(),
                ImapMailboxSource::View(_) => "0".into(),
            };
            let smart = "local.kind = 'smart'";
            let trashed = "EXISTS(SELECT 1 FROM imap_message_placements WHERE account_identifier = box.account_identifier AND mail_sequence = mail.sequence AND target_kind = 'trash') OR EXISTS(SELECT 1 FROM imap_message_copies WHERE account_identifier = box.account_identifier AND mail_sequence = mail.sequence AND target_kind = 'trash')";
            let copy = "EXISTS(SELECT 1 FROM imap_message_copies WHERE account_identifier = box.account_identifier AND mail_sequence = mail.sequence AND target_kind = box.source AND folder_identifier IS box.folder_identifier)";
            format!(
                "({retained}) AND (({smart} AND NOT ({trashed})) OR (coalesce(local.kind, '') != 'smart' AND (({primary}) OR ({copy}))))"
            )
        }
    }

    /// Map a mailbox folder projection to its primary or account-local definition.
    pub(super) async fn target(
        &self,
        connection: &mut SqliteConnection,
        source: ImapMailboxSource,
    ) -> Result<Target, StorageError> {
        match source {
            ImapMailboxSource::Inbox => Ok(Target::fixed("inbox")),
            ImapMailboxSource::Trash => Ok(Target::fixed("trash")),
            ImapMailboxSource::View(_) => Err(StorageError::ConstraintViolation(
                "smart folders cannot be organized".into(),
            )),
            ImapMailboxSource::LocalFolder(folder) => {
                let row = sqlx::query("SELECT kind, definition_identifier FROM imap_local_folders WHERE identifier = ?1 AND account_identifier = ?2")
                    .bind(folder.to_string()).bind(&self.account).fetch_optional(connection).await.storage()?.ok_or(StorageError::NotFound)?;
                if row.try_get::<String, _>("kind").storage()? == "smart" {
                    return Err(StorageError::ConstraintViolation(
                        "smart folders cannot be organized".into(),
                    ));
                }
                Ok(Target {
                    kind: "folder".into(),
                    folder: if self.mirror {
                        row.try_get("definition_identifier").storage()?
                    } else {
                        Some(folder.to_string())
                    },
                })
            }
        }
    }

    /// Current primary occurrence, with Inbox as the implicit fallback.
    pub(super) async fn primary(
        &self,
        connection: &mut SqliteConnection,
        sequence: i64,
    ) -> Result<Target, StorageError> {
        if self.mirror {
            let trash: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_message_trash WHERE owner_identifier = ?1 AND mail_sequence = ?2)").bind(&self.identifier).bind(sequence).fetch_one(&mut *connection).await.storage()?;
            if trash {
                return Ok(Target::fixed("trash"));
            }
            let folder: Option<String> = sqlx::query_scalar("SELECT folder_identifier FROM user_message_placements WHERE owner_identifier = ?1 AND mail_sequence = ?2").bind(&self.identifier).bind(sequence).fetch_optional(connection).await.storage()?.flatten();
            Ok(folder.map_or_else(
                || Target::fixed("inbox"),
                |folder| Target {
                    kind: "folder".into(),
                    folder: Some(folder),
                },
            ))
        } else {
            Ok(sqlx::query_as::<_,(String,Option<String>)>("SELECT target_kind,folder_identifier FROM imap_message_placements WHERE account_identifier = ?1 AND mail_sequence = ?2").bind(&self.identifier).bind(sequence).fetch_optional(connection).await.storage()?.map_or_else(|| Target::fixed("inbox"), |(kind,folder)| Target {kind,folder}))
        }
    }

    /// Durable additional occurrences in deterministic normal-before-Trash order.
    pub(super) async fn copies(
        &self,
        connection: &mut SqliteConnection,
        sequence: i64,
    ) -> Result<Vec<Target>, StorageError> {
        let (table, column) = if self.mirror {
            ("user_message_copies", "owner_identifier")
        } else {
            ("imap_message_copies", "account_identifier")
        };
        let sql = format!(
            "SELECT target_kind,folder_identifier FROM {table} WHERE {column} = ?1 AND mail_sequence = ?2 ORDER BY target_kind = 'trash', rowid DESC"
        );
        Ok(
            sqlx::query_as::<_, (String, Option<String>)>(sqlx::AssertSqlSafe(sql))
                .bind(&self.identifier)
                .bind(sequence)
                .fetch_all(connection)
                .await
                .storage()?
                .into_iter()
                .map(|(kind, folder)| Target { kind, folder })
                .collect(),
        )
    }

    /// Add or forget an additional occurrence without changing the primary occurrence.
    pub(super) async fn copy_record(
        &self,
        connection: &mut SqliteConnection,
        sequence: i64,
        target: &Target,
        insert: bool,
    ) -> Result<(), StorageError> {
        let (table, column) = if self.mirror {
            ("user_message_copies", "owner_identifier")
        } else {
            ("imap_message_copies", "account_identifier")
        };
        let sql = if insert {
            format!(
                "INSERT OR IGNORE INTO {table}({column},mail_sequence,target_kind,folder_identifier) VALUES(?1,?2,?3,?4)"
            )
        } else {
            format!(
                "DELETE FROM {table} WHERE {column}=?1 AND mail_sequence=?2 AND target_kind=?3 AND folder_identifier IS ?4"
            )
        };
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&self.identifier)
            .bind(sequence)
            .bind(&target.kind)
            .bind(&target.folder)
            .execute(connection)
            .await
            .storage()?;
        Ok(())
    }

    /// Replace primary placement, clearing primary Trash when restoring normal organization.
    pub(super) async fn place(
        &self,
        connection: &mut SqliteConnection,
        sequence: i64,
        target: &Target,
    ) -> Result<(), StorageError> {
        if self.mirror {
            sqlx::query("DELETE FROM user_message_placements WHERE owner_identifier=?1 AND mail_sequence=?2").bind(&self.identifier).bind(sequence).execute(&mut *connection).await.storage()?;
            sqlx::query(
                "DELETE FROM user_message_trash WHERE owner_identifier=?1 AND mail_sequence=?2",
            )
            .bind(&self.identifier)
            .bind(sequence)
            .execute(&mut *connection)
            .await
            .storage()?;
            if target.kind == "trash" {
                sqlx::query("INSERT INTO user_message_trash(owner_identifier,mail_sequence,trashed_at) VALUES(?1,?2,?3)").bind(&self.identifier).bind(sequence).bind(super::shared::now_unix()).execute(&mut *connection).await.storage()?;
            } else {
                sqlx::query("INSERT INTO user_message_placements(owner_identifier,mail_sequence,folder_identifier) VALUES(?1,?2,?3)").bind(&self.identifier).bind(sequence).bind(&target.folder).execute(&mut *connection).await.storage()?;
            }
        } else {
            sqlx::query("INSERT INTO imap_message_placements(account_identifier,mail_sequence,target_kind,folder_identifier) VALUES(?1,?2,?3,?4) ON CONFLICT(account_identifier,mail_sequence) DO UPDATE SET target_kind=excluded.target_kind,folder_identifier=excluded.folder_identifier").bind(&self.identifier).bind(sequence).bind(&target.kind).bind(&target.folder).execute(&mut *connection).await.storage()?;
        }
        Ok(())
    }

    /// Remove one occurrence, retaining surviving copies or applying two-stage Trash deletion.
    pub(super) async fn remove(
        &self,
        connection: &mut SqliteConnection,
        sequence: i64,
        target: &Target,
    ) -> Result<(), StorageError> {
        let primary = self.primary(connection, sequence).await?;
        self.copy_record(connection, sequence, target, false)
            .await?;
        if primary != *target {
            return Ok(());
        }
        let copies = self.copies(connection, sequence).await?;
        if let Some(survivor) = copies.first() {
            self.copy_record(connection, sequence, survivor, false)
                .await?;
            self.place(connection, sequence, survivor).await?;
        } else if target.kind != "trash" {
            self.place(connection, sequence, &Target::fixed("trash"))
                .await?;
        } else if self.mirror {
            sqlx::query("INSERT OR IGNORE INTO user_message_deletions(user_identifier,mail_sequence) VALUES(?1,?2)").bind(&self.identifier).bind(sequence).execute(&mut *connection).await.storage()?;
        } else {
            for table in [
                "imap_message_placements",
                "imap_message_copies",
                "imap_message_flags",
                "imap_account_messages",
            ] {
                let sql =
                    format!("DELETE FROM {table} WHERE account_identifier=?1 AND mail_sequence=?2");
                sqlx::query(sqlx::AssertSqlSafe(sql))
                    .bind(&self.identifier)
                    .bind(sequence)
                    .execute(&mut *connection)
                    .await
                    .storage()?;
            }
            sqlx::query("DELETE FROM imap_mailbox_messages WHERE mail_sequence=?1 AND mailbox_identifier IN(SELECT identifier FROM imap_mailboxes WHERE account_identifier=?2 AND organization_mode='independent')").bind(sequence).bind(&self.account).execute(&mut *connection).await.storage()?;
        }
        Ok(())
    }

    /// Invalidate all projections of the changed view, including other Mirror credentials.
    pub(super) async fn invalidate(
        &self,
        connection: &mut SqliteConnection,
    ) -> Result<(), StorageError> {
        let sql = if self.mirror {
            "UPDATE imap_mailboxes SET definition_fingerprint=NULL WHERE organization_mode='mirror' AND account_identifier IN(SELECT identifier FROM imap_accounts WHERE owner_identifier=?1)"
        } else {
            "UPDATE imap_mailboxes SET definition_fingerprint=NULL WHERE organization_mode='independent' AND account_identifier=?1"
        };
        sqlx::query(sql)
            .bind(&self.identifier)
            .execute(connection)
            .await
            .storage()?;
        Ok(())
    }
}

/// Resolve state ownership from the mailbox's durable mode, rejecting inactive handles.
pub(super) async fn organization_for_mailbox(
    connection: &mut SqliteConnection,
    mailbox: ImapMailboxIdentifier,
) -> Result<Organization, StorageError> {
    let row=sqlx::query("SELECT account.identifier,account.owner_identifier,box.organization_mode FROM imap_mailboxes AS box JOIN imap_accounts AS account ON account.identifier=box.account_identifier WHERE box.identifier=?1 AND box.organization_mode=account.mode").bind(mailbox.to_string()).fetch_optional(connection).await.storage()?.ok_or(StorageError::NotFound)?;
    let mirror = row.try_get::<String, _>("organization_mode").storage()? == "mirror";
    let account: String = row.try_get("identifier").storage()?;
    Ok(Organization {
        mirror,
        identifier: if mirror {
            row.try_get("owner_identifier").storage()?
        } else {
            account.clone()
        },
        account,
    })
}

/// Remove folder occurrences before deleting a definition, preserving unrelated normal copies.
pub(super) async fn delete_folder_occurrences(
    connection: &mut SqliteConnection,
    account: ImapAccountIdentifier,
    folder: ImapFolderIdentifier,
) -> Result<(), StorageError> {
    let row=sqlx::query("SELECT account.owner_identifier,local.organization_mode,local.definition_identifier FROM imap_local_folders AS local JOIN imap_accounts AS account ON account.identifier=local.account_identifier WHERE local.identifier=?1 AND local.account_identifier=?2").bind(folder.to_string()).bind(account.to_string()).fetch_optional(&mut *connection).await.storage()?.ok_or(StorageError::NotFound)?;
    let mirror = row.try_get::<String, _>("organization_mode").storage()? == "mirror";
    let organization = Organization {
        mirror,
        identifier: if mirror {
            row.try_get("owner_identifier").storage()?
        } else {
            account.to_string()
        },
        account: account.to_string(),
    };
    let target = Target {
        kind: "folder".into(),
        folder: if mirror {
            row.try_get("definition_identifier").storage()?
        } else {
            Some(folder.to_string())
        },
    };
    let sql = if mirror {
        "SELECT mail_sequence FROM user_message_placements WHERE owner_identifier=?1 AND folder_identifier=?2 UNION SELECT mail_sequence FROM user_message_copies WHERE owner_identifier=?1 AND folder_identifier=?2"
    } else {
        "SELECT mail_sequence FROM imap_message_placements WHERE account_identifier=?1 AND folder_identifier=?2 UNION SELECT mail_sequence FROM imap_message_copies WHERE account_identifier=?1 AND folder_identifier=?2"
    };
    let sequences: Vec<i64> = sqlx::query_scalar(sql)
        .bind(&organization.identifier)
        .bind(&target.folder)
        .fetch_all(&mut *connection)
        .await
        .storage()?;
    for sequence in sequences {
        organization.remove(connection, sequence, &target).await?;
    }
    organization.invalidate(connection).await
}

/// Collect mail released by every primary user and independent view in the current transaction.
///
/// Primary users implicitly retain all non-tombstoned mail, conservatively including mail
/// temporarily outside their scope. Independent references are retained while dormant or denied.
/// Foreign-key cascades remove facts/attachments; the existing mail trigger queues index deletion.
pub(crate) async fn collect_unreferenced_messages(
    connection: &mut SqliteConnection,
) -> Result<u64, StorageError> {
    // ponytail: scan on reference release; restrict to released sequences if large pools make it costly.
    Ok(sqlx::query("DELETE FROM mail WHERE EXISTS(SELECT 1 FROM users) AND NOT EXISTS(SELECT 1 FROM users WHERE NOT EXISTS(SELECT 1 FROM user_message_deletions WHERE user_identifier=users.identifier AND mail_sequence=mail.sequence)) AND NOT EXISTS(SELECT 1 FROM imap_account_messages WHERE mail_sequence=mail.sequence)").execute(connection).await.storage()?.rows_affected())
}

/// Apply normal-folder deletion to the primary state even when no Mirror credential exists.
pub(super) async fn delete_primary_folder_occurrences(
    connection: &mut SqliteConnection,
    owner: sandpost_core::UserIdentifier,
    folder: ImapFolderIdentifier,
) -> Result<(), StorageError> {
    let organization = Organization {
        mirror: true,
        identifier: owner.to_string(),
        account: String::new(),
    };
    let target = Target {
        kind: "folder".into(),
        folder: Some(folder.to_string()),
    };
    let sequences:Vec<i64>=sqlx::query_scalar("SELECT mail_sequence FROM user_message_placements WHERE owner_identifier=?1 AND folder_identifier=?2 UNION SELECT mail_sequence FROM user_message_copies WHERE owner_identifier=?1 AND folder_identifier=?2").bind(&organization.identifier).bind(&target.folder).fetch_all(&mut *connection).await.storage()?;
    for sequence in sequences {
        organization.remove(connection, sequence, &target).await?;
    }
    organization.invalidate(connection).await
}

/// Rename all descendants atomically while retaining folder identifiers and rejecting collisions.
pub(super) async fn rename_folder_descendants(
    connection: &mut SqliteConnection,
    primary: bool,
    owner: &str,
    previous: &str,
    replacement: &str,
) -> Result<(), StorageError> {
    if previous == replacement {
        return Ok(());
    }
    if replacement.starts_with(&format!("{previous}/")) {
        return Err(StorageError::ConstraintViolation(
            "a folder cannot be renamed beneath itself".into(),
        ));
    }
    let (table, column, restriction) = if primary {
        ("web_folders", "owner_identifier", "")
    } else {
        (
            "imap_local_folders",
            "account_identifier",
            " AND organization_mode='independent'",
        )
    };
    let sql = format!("SELECT identifier,name FROM {table} WHERE {column}=?1{restriction}");
    let rows: Vec<(String, String)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(owner)
        .fetch_all(&mut *connection)
        .await
        .storage()?;
    let prefix = format!("{previous}/");
    let descendants: Vec<_> = rows
        .iter()
        .filter(|(_, name)| name.starts_with(&prefix))
        .collect();
    for (identifier, name) in &descendants {
        let wanted = format!("{replacement}{}", &name[previous.len()..]);
        if rows.iter().any(|(other_identifier, other_name)| {
            other_identifier != identifier
                && !other_name.starts_with(&prefix)
                && other_name.eq_ignore_ascii_case(&wanted)
        }) {
            return Err(StorageError::DuplicateImapFolderName);
        }
    }
    for (identifier, name) in descendants {
        let sql = format!("UPDATE {table} SET name=?1,updated_at=?2 WHERE identifier=?3");
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(format!("{replacement}{}", &name[previous.len()..]))
            .bind(super::shared::now_unix())
            .bind(identifier)
            .execute(&mut *connection)
            .await
            .storage()?;
    }
    Ok(())
}
