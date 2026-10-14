ALTER TABLE imap_local_folders ADD COLUMN organization_mode TEXT NOT NULL DEFAULT 'independent'
    CHECK (organization_mode IN ('mirror', 'independent'));
