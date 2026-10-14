ALTER TABLE imap_mailboxes ADD COLUMN organization_mode TEXT NOT NULL DEFAULT 'independent'
    CHECK (organization_mode IN ('mirror', 'independent'));
UPDATE imap_mailboxes SET organization_mode = 'mirror'
WHERE folder_identifier IN (SELECT identifier FROM imap_local_folders WHERE organization_mode = 'mirror');
DROP INDEX imap_mailboxes_fixed_source;
DROP INDEX imap_mailboxes_view;
CREATE UNIQUE INDEX imap_mailboxes_fixed_source
ON imap_mailboxes(account_identifier, organization_mode, source) WHERE source IN ('inbox', 'trash');
CREATE UNIQUE INDEX imap_mailboxes_view
ON imap_mailboxes(account_identifier, organization_mode, view_identifier) WHERE view_identifier IS NOT NULL;
