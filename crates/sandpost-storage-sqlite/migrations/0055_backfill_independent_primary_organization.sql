-- Preserve the Web organization previously inherited by existing accounts during upgrade.
-- Original manual projection IDs remain Independent folders; new linked projections are Mirror.
INSERT OR IGNORE INTO imap_message_placements(account_identifier,mail_sequence,target_kind,folder_identifier)
SELECT account.identifier, placement.mail_sequence, 'folder', local.identifier
FROM imap_accounts AS account
JOIN user_message_placements AS placement ON placement.owner_identifier = account.owner_identifier
JOIN imap_account_messages AS retained ON retained.account_identifier=account.identifier
    AND retained.mail_sequence=placement.mail_sequence
JOIN web_folders AS definition ON definition.identifier=placement.folder_identifier
JOIN imap_local_folders AS local ON local.account_identifier=account.identifier
    AND local.organization_mode='independent' AND local.kind='private' AND local.definition_identifier=definition.identifier;
-- Exclusive account placement always won over inherited Web Trash in the former policy.
INSERT OR IGNORE INTO imap_message_placements(account_identifier,mail_sequence,target_kind)
SELECT account.identifier, trash.mail_sequence, 'trash'
FROM imap_accounts AS account
JOIN user_message_trash AS trash ON trash.owner_identifier=account.owner_identifier
JOIN imap_account_messages AS retained ON retained.account_identifier=account.identifier
    AND retained.mail_sequence=trash.mail_sequence;

-- The temporary origin keys serve upgrade conversion only; future primary edits must not inherit.
UPDATE imap_local_folders SET definition_identifier=NULL WHERE organization_mode='independent';
