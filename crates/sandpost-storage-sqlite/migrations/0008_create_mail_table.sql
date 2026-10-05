-- Authoritative mail and its immutable identity and ordering keys.
CREATE TABLE mail (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    identifier TEXT NOT NULL UNIQUE,
    subject TEXT NOT NULL,
    text_body TEXT NOT NULL,
    markup_body TEXT NOT NULL,
    message_identifier TEXT,
    raw_message BLOB NOT NULL,
    received_at INTEGER NOT NULL,
    size INTEGER NOT NULL CHECK (size >= 0),
    search_revision INTEGER NOT NULL DEFAULT 0
        CHECK (typeof(search_revision) = 'integer' AND search_revision >= 0),
    endpoint_identifier TEXT NOT NULL REFERENCES endpoints(identifier) ON DELETE RESTRICT
);

CREATE INDEX mail_received ON mail(received_at DESC, sequence DESC);
CREATE INDEX mail_endpoint_sequence ON mail(endpoint_identifier, sequence);
