-- Global SMTP access credentials and authentication rules.
CREATE TABLE smtp_accesses (
    identifier TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    requires_encryption INTEGER NOT NULL DEFAULT 0 CHECK (requires_encryption IN (0, 1)),
    plain_mechanism_allowed INTEGER NOT NULL DEFAULT 1 CHECK (plain_mechanism_allowed IN (0, 1)),
    login_mechanism_allowed INTEGER NOT NULL DEFAULT 1 CHECK (login_mechanism_allowed IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_used_at INTEGER
);

CREATE INDEX smtp_accesses_name ON smtp_accesses(name, identifier);
