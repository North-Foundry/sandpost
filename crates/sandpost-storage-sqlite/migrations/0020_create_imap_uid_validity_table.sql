-- Allocator of IMAP UIDVALIDITY values. Every mailbox identity receives a value greater than any
-- value handed out before, so a mailbox recreated under an old name never reuses UIDVALIDITY.
CREATE TABLE imap_uid_validity (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    last_value INTEGER NOT NULL CHECK (last_value BETWEEN 0 AND 4294967295)
);

INSERT INTO imap_uid_validity(singleton, last_value) VALUES (1, 0);
