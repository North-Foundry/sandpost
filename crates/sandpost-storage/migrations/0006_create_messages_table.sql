
CREATE TABLE messages (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    facts TEXT NOT NULL,
    raw_mime BLOB NOT NULL,
    attachments TEXT NOT NULL,
    sender_domain TEXT NOT NULL,
    received_at INTEGER NOT NULL,
    subject TEXT NOT NULL DEFAULT '',
    from_json TEXT NOT NULL DEFAULT '[]',
    to_json TEXT NOT NULL DEFAULT '[]',
    size INTEGER NOT NULL DEFAULT 0,
    attachment_count INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX messages_received ON messages(received_at DESC, seq DESC);
CREATE INDEX messages_sender_domain ON messages(sender_domain);
