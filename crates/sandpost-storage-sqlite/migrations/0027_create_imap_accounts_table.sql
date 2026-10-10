-- Schema version 5: user-owned IMAP accounts. Each account belongs to one Sandpost user and never
-- reads beyond that user's mail visibility. Only an Argon2id PHC hash of the password is stored.
-- `flag_modification` is the account's flag-change counter, used as the IMAP synchronization
-- cursor for flags shared by all of its mailboxes.
CREATE TABLE imap_accounts (
    identifier TEXT PRIMARY KEY,
    owner_identifier TEXT NOT NULL REFERENCES users(identifier) ON DELETE CASCADE,
    name TEXT NOT NULL,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    mirror_views INTEGER NOT NULL DEFAULT 1 CHECK (mirror_views IN (0, 1)),
    flag_modification INTEGER NOT NULL DEFAULT 0 CHECK (flag_modification >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_used_at INTEGER
);

CREATE INDEX imap_accounts_owner ON imap_accounts(owner_identifier, name, identifier);
