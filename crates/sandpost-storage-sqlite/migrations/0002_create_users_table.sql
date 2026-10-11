-- Canonical users with independent global roles and mail access.
CREATE TABLE users (
    identifier TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    global_role TEXT NOT NULL CHECK (global_role IN ('owner', 'admin', 'member', 'viewer')),
    mail_access TEXT NOT NULL CHECK (mail_access IN ('all', 'scoped')),
    personal_filter TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
