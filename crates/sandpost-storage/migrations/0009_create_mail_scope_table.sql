
CREATE TABLE mail_scope (
    scope_identifier TEXT NOT NULL REFERENCES scopes(identifier) ON DELETE CASCADE,
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    policy_version INTEGER NOT NULL CHECK (policy_version >= 0),
    PRIMARY KEY (scope_identifier, mail_sequence)
);

CREATE INDEX mail_scope_sequence ON mail_scope(mail_sequence, scope_identifier);
