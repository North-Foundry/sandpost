-- Account folders and Mirror projections. Smart filters use query-language source
-- evaluated over the owner's visible mail; Independent definitions stay account-local.
CREATE TABLE imap_local_folders (
    identifier TEXT PRIMARY KEY,
    account_identifier TEXT NOT NULL REFERENCES imap_accounts(identifier) ON DELETE CASCADE,
    name TEXT NOT NULL,
    filter TEXT NOT NULL,
    include_in_inbox INTEGER NOT NULL DEFAULT 1 CHECK (include_in_inbox IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    kind TEXT NOT NULL DEFAULT 'smart' CHECK (kind IN ('smart', 'manual', 'private')),
    definition_identifier TEXT,
    organization_mode TEXT NOT NULL DEFAULT 'independent'
        CHECK (organization_mode IN ('mirror', 'independent')),
    UNIQUE (account_identifier, name)
);
