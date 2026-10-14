-- Preserve original folder IDs, membership, flags and placements as Independent state.
-- New projections alone represent the canonical primary definitions.
CREATE TEMP TABLE imap_mode_projection_backfill AS
SELECT local.identifier AS original_identifier,
       lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-' || hex(randomblob(2)) || '-' || hex(randomblob(2)) || '-' || hex(randomblob(6))) AS mirror_identifier,
       local.account_identifier, coalesce(web.name, definition.name, local.name) AS original_name
FROM imap_local_folders AS local
LEFT JOIN web_folder_accounts AS web_link ON web_link.local_folder_identifier = local.identifier
LEFT JOIN web_folders AS web ON web.identifier = web_link.folder_identifier
LEFT JOIN imap_user_folder_accounts AS link ON link.local_folder_identifier = local.identifier
LEFT JOIN imap_user_folders AS definition ON definition.identifier = link.folder_identifier
WHERE web_link.local_folder_identifier IS NOT NULL OR link.local_folder_identifier IS NOT NULL;
INSERT INTO imap_local_folders(identifier, account_identifier, name, filter, include_in_inbox,
    created_at, updated_at, kind, definition_identifier, organization_mode)
SELECT map.mirror_identifier, local.account_identifier, '__sandpost_mode_projection_' || map.mirror_identifier, local.filter,
       local.include_in_inbox, local.created_at, local.updated_at, local.kind,
       local.definition_identifier, 'mirror'
FROM imap_local_folders AS local JOIN imap_mode_projection_backfill AS map
ON map.original_identifier = local.identifier;
UPDATE web_folder_accounts SET local_folder_identifier =
    (SELECT mirror_identifier FROM imap_mode_projection_backfill WHERE original_identifier = web_folder_accounts.local_folder_identifier);
UPDATE imap_user_folder_accounts SET local_folder_identifier =
    (SELECT mirror_identifier FROM imap_mode_projection_backfill WHERE original_identifier = imap_user_folder_accounts.local_folder_identifier);
UPDATE imap_local_folders SET name =
    (SELECT CASE WHEN (SELECT count(*) FROM imap_mode_projection_backfill AS other
           WHERE other.account_identifier = map.account_identifier AND other.original_name = map.original_name) = 1
           AND NOT EXISTS(SELECT 1 FROM imap_local_folders AS other WHERE other.account_identifier = map.account_identifier
               AND other.name = map.original_name AND other.identifier != map.original_identifier)
       THEN original_name ELSE original_name || ' (' || original_identifier || ')' END
     FROM imap_mode_projection_backfill AS map WHERE original_identifier = imap_local_folders.identifier),
    kind = CASE WHEN kind = 'manual' THEN 'private' ELSE kind END
WHERE identifier IN(SELECT original_identifier FROM imap_mode_projection_backfill);
DROP TABLE imap_mode_projection_backfill;

