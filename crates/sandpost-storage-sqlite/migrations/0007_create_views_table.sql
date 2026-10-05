-- Saved views: personal when owned by a user, shared when the owner is null.
CREATE TABLE views (
    identifier TEXT PRIMARY KEY,
    endpoint_identifier TEXT NOT NULL REFERENCES endpoints(identifier) ON DELETE RESTRICT,
    owner_identifier TEXT REFERENCES users(identifier) ON DELETE CASCADE,
    name TEXT NOT NULL,
    filter TEXT NOT NULL
);

CREATE INDEX views_owner_name ON views(owner_identifier, name, identifier);
