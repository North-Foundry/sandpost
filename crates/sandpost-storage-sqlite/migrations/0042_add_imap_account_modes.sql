-- Existing credentials keep their former account-local organization on upgrade.
ALTER TABLE imap_accounts ADD COLUMN mode TEXT NOT NULL DEFAULT 'independent'
    CHECK (mode IN ('mirror', 'independent'));
ALTER TABLE imap_accounts ADD COLUMN mode_generation INTEGER NOT NULL DEFAULT 0;
