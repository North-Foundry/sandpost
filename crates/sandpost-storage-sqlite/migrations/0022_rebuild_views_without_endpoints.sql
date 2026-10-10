-- Saved views filter the single mail pool; their endpoint column is removed.
CREATE TABLE views_rebuilt (
    identifier TEXT PRIMARY KEY,
    owner_identifier TEXT REFERENCES users(identifier) ON DELETE CASCADE,
    name TEXT NOT NULL,
    filter TEXT NOT NULL
);

INSERT INTO views_rebuilt(identifier, owner_identifier, name, filter)
SELECT identifier, owner_identifier, name, filter FROM views;

DROP TABLE views;

ALTER TABLE views_rebuilt RENAME TO views;

CREATE INDEX views_owner_name ON views(owner_identifier, name, identifier);
