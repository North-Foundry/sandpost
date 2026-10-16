-- Require Web folder exposures to use matching owner and manual projections.
CREATE TRIGGER web_folder_accounts_ownership_insert
BEFORE INSERT ON web_folder_accounts
WHEN NOT EXISTS (
    SELECT 1 FROM web_folders AS folder
    JOIN imap_accounts AS account ON account.owner_identifier = folder.owner_identifier
    JOIN imap_local_folders AS local ON local.account_identifier = account.identifier
    WHERE folder.identifier = NEW.folder_identifier
      AND account.identifier = NEW.account_identifier
      AND local.identifier = NEW.local_folder_identifier
      AND local.kind = 'manual' AND local.definition_identifier = folder.identifier
)
BEGIN SELECT RAISE(ABORT, 'folder exposure ownership does not match'); END;

CREATE TRIGGER web_folder_accounts_ownership_update
BEFORE UPDATE ON web_folder_accounts
WHEN NOT EXISTS (
    SELECT 1 FROM web_folders AS folder
    JOIN imap_accounts AS account ON account.owner_identifier = folder.owner_identifier
    JOIN imap_local_folders AS local ON local.account_identifier = account.identifier
    WHERE folder.identifier = NEW.folder_identifier
      AND account.identifier = NEW.account_identifier
      AND local.identifier = NEW.local_folder_identifier
      AND local.kind = 'manual' AND local.definition_identifier = folder.identifier
)
BEGIN SELECT RAISE(ABORT, 'folder exposure ownership does not match'); END;
