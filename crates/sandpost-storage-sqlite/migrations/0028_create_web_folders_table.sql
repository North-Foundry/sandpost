-- Owner-managed manual folders exposed through the Web interface.
CREATE TABLE web_folders (
    identifier TEXT PRIMARY KEY,
    owner_identifier TEXT NOT NULL REFERENCES users(identifier) ON DELETE CASCADE,
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX web_folders_owner_name ON web_folders(owner_identifier, name);
