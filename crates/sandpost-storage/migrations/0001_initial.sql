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

CREATE TABLE users (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    personal_filter TEXT
);
CREATE TABLE memberships (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    scope_id TEXT NOT NULL REFERENCES scopes(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member', 'viewer')),
    PRIMARY KEY (user_id, scope_id)
);
CREATE INDEX memberships_scope ON memberships(scope_id, user_id);
CREATE TABLE inboxes (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    filter TEXT NOT NULL
);
CREATE INDEX inboxes_user ON inboxes(user_id, name);

CREATE TABLE messages (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    facts TEXT NOT NULL,
    raw_mime BLOB NOT NULL,
    attachments TEXT NOT NULL,
    sender_domain TEXT NOT NULL,
    received_at INTEGER NOT NULL
);
CREATE INDEX messages_received ON messages(received_at DESC, seq DESC);
CREATE INDEX messages_sender_domain ON messages(sender_domain);
CREATE TABLE message_recipients (
    message_seq INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
    address TEXT NOT NULL,
    domain TEXT NOT NULL,
    PRIMARY KEY (message_seq, address)
);
CREATE INDEX message_recipients_domain ON message_recipients(domain, message_seq);
CREATE TABLE message_headers (
    message_seq INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
    name TEXT NOT NULL,
    value TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    PRIMARY KEY (message_seq, name, ordinal)
);
CREATE INDEX message_headers_lookup ON message_headers(name, value, message_seq);
CREATE TABLE message_scope (
    scope_id TEXT NOT NULL REFERENCES scopes(id) ON DELETE CASCADE,
    message_seq INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
    policy_version INTEGER NOT NULL CHECK (policy_version >= 0),
    PRIMARY KEY (scope_id, message_seq)
);
CREATE INDEX message_scope_sequence ON message_scope(message_seq, scope_id);
