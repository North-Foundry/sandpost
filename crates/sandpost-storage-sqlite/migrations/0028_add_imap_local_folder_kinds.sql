-- Distinguish smart, manual, and account-private folder projections.
ALTER TABLE imap_local_folders ADD COLUMN kind TEXT NOT NULL DEFAULT 'smart'
    CHECK (kind IN ('smart', 'manual', 'private'));

ALTER TABLE imap_local_folders ADD COLUMN definition_identifier TEXT;

UPDATE imap_local_folders
SET definition_identifier = (
    SELECT folder_identifier FROM imap_user_folder_accounts
    WHERE local_folder_identifier = imap_local_folders.identifier
);
