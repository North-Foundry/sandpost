-- Owner smart-folder definitions survive deletion of individual IMAP credentials.
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
