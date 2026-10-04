
CREATE TABLE memberships (
    user_identifier TEXT NOT NULL REFERENCES users(identifier) ON DELETE CASCADE,
    scope_identifier TEXT NOT NULL REFERENCES scopes(identifier) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member', 'viewer')),
    PRIMARY KEY (user_identifier, scope_identifier)
);

CREATE INDEX memberships_scope ON memberships(scope_identifier, user_identifier);
