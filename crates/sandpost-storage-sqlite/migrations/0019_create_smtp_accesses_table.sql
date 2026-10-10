-- Centrally administered SMTP AUTH credentials. Only an Argon2id PHC hash of each generated
-- password is stored; the plaintext is never persisted. Mail submitted with an access is captured
-- for its endpoint, so an endpoint cannot be deleted while an access targets it.
CREATE TABLE smtp_accesses (
    identifier TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    endpoint_identifier TEXT NOT NULL REFERENCES endpoints(identifier) ON DELETE RESTRICT,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_used_at INTEGER
);

CREATE INDEX smtp_accesses_endpoint ON smtp_accesses(endpoint_identifier);
CREATE INDEX smtp_accesses_name ON smtp_accesses(name, identifier);
