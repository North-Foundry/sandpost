-- SMTP accesses deliver into the single mail pool; their endpoint column is removed.
CREATE TABLE smtp_accesses_rebuilt (
    identifier TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_used_at INTEGER
);

INSERT INTO smtp_accesses_rebuilt(
    identifier, name, username, password_hash, enabled, created_at, updated_at, last_used_at
)
SELECT identifier, name, username, password_hash, enabled, created_at, updated_at, last_used_at
FROM smtp_accesses;

DROP TABLE smtp_accesses;

ALTER TABLE smtp_accesses_rebuilt RENAME TO smtp_accesses;

CREATE INDEX smtp_accesses_name ON smtp_accesses(name, identifier);
