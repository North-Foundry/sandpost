-- Link owner smart-folder definitions to their account-local Mirror projections.
CREATE TABLE imap_user_folder_accounts (
    account_identifier TEXT NOT NULL REFERENCES imap_accounts(identifier) ON DELETE CASCADE,
    folder_identifier TEXT NOT NULL REFERENCES imap_user_folders(identifier) ON DELETE CASCADE,
    local_folder_identifier TEXT NOT NULL UNIQUE
        REFERENCES imap_local_folders(identifier) ON DELETE CASCADE,
    PRIMARY KEY (account_identifier, folder_identifier)
);
