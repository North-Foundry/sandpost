-- Attachment metadata; bytes are recovered from the raw MIME message.
CREATE TABLE mail_attachments (
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    filename TEXT,
    content_type TEXT NOT NULL,
    size INTEGER NOT NULL CHECK (size >= 0),
    content_hash TEXT NOT NULL,
    PRIMARY KEY (mail_sequence, ordinal)
);
