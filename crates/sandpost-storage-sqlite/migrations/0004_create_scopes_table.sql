-- Hierarchical mail scopes; every scope belongs to exactly one endpoint.
CREATE TABLE scopes (
    identifier TEXT PRIMARY KEY,
    parent_identifier TEXT REFERENCES scopes(identifier),
    name TEXT NOT NULL,
    description TEXT,
    filter TEXT NOT NULL,
    position INTEGER NOT NULL,
    policy_version INTEGER NOT NULL CHECK (policy_version >= 0),
    endpoint_identifier TEXT NOT NULL REFERENCES endpoints(identifier)
);

CREATE INDEX scopes_parent_position ON scopes(parent_identifier, position, identifier);
CREATE INDEX scopes_endpoint_position ON scopes(endpoint_identifier, position, identifier);
