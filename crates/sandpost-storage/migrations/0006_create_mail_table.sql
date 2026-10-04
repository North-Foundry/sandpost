
CREATE TABLE mail (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    identifier TEXT NOT NULL UNIQUE,
    subject TEXT NOT NULL,
    text_body TEXT NOT NULL,
    markup_body TEXT NOT NULL,
    message_identifier TEXT,
    raw_message BLOB NOT NULL,
    received_at INTEGER NOT NULL,
    size INTEGER NOT NULL CHECK (size >= 0)
);

CREATE INDEX mail_received ON mail(received_at DESC, sequence DESC);
