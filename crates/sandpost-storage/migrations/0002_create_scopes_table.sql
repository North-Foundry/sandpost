
CREATE TABLE scopes (
    id TEXT PRIMARY KEY,
    parent_id TEXT REFERENCES scopes(id),
    name TEXT NOT NULL,
    description TEXT,
    filter TEXT NOT NULL,
    position INTEGER NOT NULL,
    policy_version INTEGER NOT NULL CHECK (policy_version >= 0)
);

CREATE INDEX scopes_parent_position ON scopes(parent_id, position, id);
