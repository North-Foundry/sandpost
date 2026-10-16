-- Role-less assignment of a user to a mail subset over the mail pool.
CREATE TABLE scope_memberships (
    user_identifier TEXT NOT NULL REFERENCES users(identifier) ON DELETE CASCADE,
    scope_identifier TEXT NOT NULL REFERENCES scopes(identifier) ON DELETE CASCADE,
    PRIMARY KEY (user_identifier, scope_identifier)
);

CREATE INDEX scope_memberships_scope ON scope_memberships(scope_identifier, user_identifier);
