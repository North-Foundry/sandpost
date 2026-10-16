-- Selected IMAP account projections of Web-managed folders.
CREATE TABLE web_folder_accounts (
    folder_identifier TEXT NOT NULL REFERENCES web_folders(identifier) ON DELETE CASCADE,
    account_identifier TEXT NOT NULL REFERENCES imap_accounts(identifier) ON DELETE CASCADE,
    local_folder_identifier TEXT NOT NULL UNIQUE
        REFERENCES imap_local_folders(identifier) ON DELETE CASCADE,
    PRIMARY KEY (folder_identifier, account_identifier)
);

CREATE INDEX web_folder_accounts_account ON web_folder_accounts(account_identifier);
