-- User-owned virtual recipients. The address is stored normalized and is unique across the
-- installation, so at most one Persona ever answers for a recipient address.
CREATE TABLE personas (
    identifier TEXT PRIMARY KEY,
    owner_identifier TEXT NOT NULL REFERENCES users(identifier) ON DELETE CASCADE,
    name TEXT NOT NULL,
    icon TEXT NULL,
    address TEXT NOT NULL UNIQUE CHECK (address <> '' AND address = lower(trim(address))),
    script TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX personas_owner ON personas(owner_identifier, name);
