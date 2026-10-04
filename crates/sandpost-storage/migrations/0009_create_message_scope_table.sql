
CREATE TABLE message_scope (
    scope_id TEXT NOT NULL REFERENCES scopes(id) ON DELETE CASCADE,
    message_seq INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
    policy_version INTEGER NOT NULL CHECK (policy_version >= 0),
    PRIMARY KEY (scope_id, message_seq)
);

CREATE INDEX message_scope_sequence ON message_scope(message_seq, scope_id);
