-- Personal and shared query Views over the mail pool.
CREATE TABLE views (
    identifier TEXT PRIMARY KEY,
    owner_identifier TEXT REFERENCES users(identifier) ON DELETE CASCADE,
    name TEXT NOT NULL,
    filter TEXT NOT NULL,
    icon TEXT NULL
);

CREATE INDEX views_owner_name ON views(owner_identifier, name, identifier);
