-- Require copy destinations to belong to the same IMAP account.
CREATE TRIGGER imap_message_copies_folder_account_insert
BEFORE INSERT ON imap_message_copies
WHEN NEW.folder_identifier IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM imap_local_folders
    WHERE identifier = NEW.folder_identifier AND account_identifier = NEW.account_identifier
)
BEGIN SELECT RAISE(ABORT, 'copy folder belongs to another account'); END;

CREATE TRIGGER imap_message_copies_folder_account_update
BEFORE UPDATE ON imap_message_copies
WHEN NEW.folder_identifier IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM imap_local_folders
    WHERE identifier = NEW.folder_identifier AND account_identifier = NEW.account_identifier
)
BEGIN SELECT RAISE(ABORT, 'copy folder belongs to another account'); END;
