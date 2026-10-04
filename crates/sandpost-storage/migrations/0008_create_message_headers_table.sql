
CREATE TABLE message_headers (
    message_seq INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
    name TEXT NOT NULL,
    value TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    PRIMARY KEY (message_seq, name, ordinal)
);

CREATE INDEX message_headers_lookup ON message_headers(name, value, message_seq);
