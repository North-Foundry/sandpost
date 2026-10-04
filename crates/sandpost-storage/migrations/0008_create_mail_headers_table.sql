
CREATE TABLE mail_headers (
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    name TEXT NOT NULL,
    value TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    PRIMARY KEY (mail_sequence, name, ordinal)
);

CREATE INDEX mail_headers_lookup ON mail_headers(name, value, mail_sequence);
