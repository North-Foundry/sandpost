-- Dynamic folders that exist only inside one IMAP account. The filter is query-language source
-- evaluated over the owner's visible mail; it never changes Sandpost Views.
CREATE TABLE imap_local_folders (
    identifier TEXT PRIMARY KEY,
    account_identifier TEXT NOT NULL REFERENCES imap_accounts(identifier) ON DELETE CASCADE,
    name TEXT NOT NULL,
    filter TEXT NOT NULL,
    include_in_inbox INTEGER NOT NULL DEFAULT 1 CHECK (include_in_inbox IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (account_identifier, name)
);
