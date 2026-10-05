-- Endpoint authority (role) kept independent from mail visibility (mail access).
CREATE TABLE endpoint_memberships (
    user_identifier TEXT NOT NULL REFERENCES users(identifier) ON DELETE CASCADE,
    endpoint_identifier TEXT NOT NULL REFERENCES endpoints(identifier) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('admin', 'member', 'viewer')),
    mail_access TEXT NOT NULL CHECK (mail_access IN ('all', 'scoped')),
    PRIMARY KEY (user_identifier, endpoint_identifier)
);

CREATE INDEX endpoint_memberships_endpoint
    ON endpoint_memberships(endpoint_identifier, user_identifier);
