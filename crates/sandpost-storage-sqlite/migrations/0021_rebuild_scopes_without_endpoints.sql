-- Scopes form one hierarchy over the single mail pool; their endpoint column is removed.
CREATE TABLE scopes_rebuilt (
    identifier TEXT PRIMARY KEY,
    parent_identifier TEXT REFERENCES scopes(identifier),
    name TEXT NOT NULL,
    description TEXT,
    filter TEXT NOT NULL,
    position INTEGER NOT NULL,
    policy_version INTEGER NOT NULL CHECK (policy_version >= 0)
);

INSERT INTO scopes_rebuilt(
    identifier, parent_identifier, name, description, filter, position, policy_version
)
SELECT identifier, parent_identifier, name, description, filter, position, policy_version
FROM scopes;

DROP TABLE scopes;

ALTER TABLE scopes_rebuilt RENAME TO scopes;

CREATE INDEX scopes_parent_position ON scopes(parent_identifier, position, identifier);
