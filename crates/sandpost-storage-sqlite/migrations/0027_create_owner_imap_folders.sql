-- Owner-scoped folder definitions survive deletion of any individual IMAP credential.
-- Existing account folders become owner definitions without merging duplicate names or filters.
CREATE TABLE imap_user_folders (
    identifier TEXT PRIMARY KEY,
    owner_identifier TEXT NOT NULL REFERENCES users(identifier) ON DELETE CASCADE,
    name TEXT NOT NULL,
    filter TEXT NOT NULL,
    include_in_inbox INTEGER NOT NULL DEFAULT 1 CHECK (include_in_inbox IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX imap_user_folders_owner
    ON imap_user_folders(owner_identifier, name, identifier);

-- Account-local rows remain mailbox projections so existing mailbox foreign keys and UIDs keep
-- their identities. The owner definition is authoritative; links and projections are disposable.
CREATE TABLE imap_user_folder_accounts (
    account_identifier TEXT NOT NULL REFERENCES imap_accounts(identifier) ON DELETE CASCADE,
    folder_identifier TEXT NOT NULL REFERENCES imap_user_folders(identifier) ON DELETE CASCADE,
    local_folder_identifier TEXT NOT NULL UNIQUE
        REFERENCES imap_local_folders(identifier) ON DELETE CASCADE,
    PRIMARY KEY (account_identifier, folder_identifier)
);

INSERT INTO imap_user_folders (
    identifier, owner_identifier, name, filter, include_in_inbox, created_at, updated_at
)
SELECT folder.identifier, account.owner_identifier, folder.name, folder.filter,
       folder.include_in_inbox, folder.created_at, folder.updated_at
FROM imap_local_folders AS folder
JOIN imap_accounts AS account ON account.identifier = folder.account_identifier;

INSERT INTO imap_user_folder_accounts (
    account_identifier, folder_identifier, local_folder_identifier
)
SELECT folder.account_identifier, folder.identifier, folder.identifier
FROM imap_local_folders AS folder;

-- Local names are internal projection keys. Canonical names come only from the owner table,
-- and owner definitions can legitimately share a legacy name across different accounts.
UPDATE imap_local_folders
SET name = '__sandpost_owner_folder_' || identifier
WHERE identifier IN (SELECT local_folder_identifier FROM imap_user_folder_accounts);

-- Preserve each original local-folder ID above, then add projections for other credentials
-- belonging to the same owner. A temporary ID map lets the insert and link share UUIDs.
CREATE TEMP TABLE imap_folder_projection_backfill AS
SELECT lower(substr(hex(randomblob(16)), 1, 8) || '-' ||
             substr(hex(randomblob(16)), 9, 4) || '-' ||
             substr(hex(randomblob(16)), 13, 4) || '-' ||
             substr(hex(randomblob(16)), 17, 4) || '-' ||
             substr(hex(randomblob(16)), 21, 12)) AS local_identifier,
       account.identifier AS account_identifier,
       definition.identifier AS folder_identifier,
       definition.filter,
       definition.include_in_inbox,
       definition.created_at,
       definition.updated_at
FROM imap_user_folders AS definition
JOIN imap_accounts AS account ON account.owner_identifier = definition.owner_identifier
WHERE NOT EXISTS (
    SELECT 1 FROM imap_user_folder_accounts AS existing
    WHERE existing.account_identifier = account.identifier
      AND existing.folder_identifier = definition.identifier
);

INSERT INTO imap_local_folders (
    identifier, account_identifier, name, filter, include_in_inbox, created_at, updated_at
)
SELECT local_identifier, account_identifier,
       '__sandpost_owner_folder_' || folder_identifier,
       filter, include_in_inbox, created_at, updated_at
FROM imap_folder_projection_backfill;

INSERT INTO imap_user_folder_accounts (
    account_identifier, folder_identifier, local_folder_identifier
)
SELECT account_identifier, folder_identifier, local_identifier
FROM imap_folder_projection_backfill;

DROP TABLE imap_folder_projection_backfill;
