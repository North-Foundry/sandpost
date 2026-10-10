-- Mail no longer records an endpoint. Identity, sequence, and search revision are copied
-- unchanged, and the AUTOINCREMENT high-water mark is preserved so deleted sequences are never
-- reused. Child-fact triggers mention the mail table, so they are dropped for the rebuild and
-- recreated with every mail trigger afterwards; copying rows enqueues no search operations.
DROP TRIGGER mail_recipients_search_revision_insert;
DROP TRIGGER mail_recipients_search_revision_delete;
DROP TRIGGER mail_recipients_search_revision_update;
DROP TRIGGER mail_headers_search_revision_insert;
DROP TRIGGER mail_headers_search_revision_delete;
DROP TRIGGER mail_headers_search_revision_update;
DROP TRIGGER mail_attachments_search_revision_insert;
DROP TRIGGER mail_attachments_search_revision_delete;
DROP TRIGGER mail_attachments_search_revision_update;

CREATE TABLE mail_sequence_high_water AS
SELECT seq FROM sqlite_sequence WHERE name = 'mail';

CREATE TABLE mail_rebuilt (
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
        CHECK (typeof(search_revision) = 'integer' AND search_revision >= 0)
);

INSERT INTO mail_rebuilt(
    sequence, identifier, subject, text_body, markup_body, message_identifier, raw_message,
    received_at, size, search_revision
)
SELECT
    sequence, identifier, subject, text_body, markup_body, message_identifier, raw_message,
    received_at, size, search_revision
FROM mail;

DROP TABLE mail;

ALTER TABLE mail_rebuilt RENAME TO mail;

INSERT INTO sqlite_sequence(name, seq)
SELECT 'mail', seq FROM mail_sequence_high_water
WHERE NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'mail');

UPDATE sqlite_sequence
SET seq = max(seq, coalesce((SELECT max(seq) FROM mail_sequence_high_water), 0))
WHERE name = 'mail';

-- Copying zero rows still records a zero high-water mark; drop it so an empty pool matches a
-- freshly created table, which has no sequence row until its first insert.
DELETE FROM sqlite_sequence WHERE name = 'mail' AND seq = 0;

DROP TABLE mail_sequence_high_water;

CREATE INDEX mail_received ON mail(received_at DESC, sequence DESC);

CREATE TRIGGER mail_identifier_immutable BEFORE UPDATE OF identifier ON mail
WHEN NEW.identifier != OLD.identifier BEGIN SELECT RAISE(ABORT, 'mail identifier is immutable'); END;

CREATE TRIGGER mail_sequence_immutable BEFORE UPDATE OF sequence ON mail
WHEN NEW.sequence != OLD.sequence BEGIN SELECT RAISE(ABORT, 'mail sequence is immutable'); END;

CREATE TRIGGER mail_search_outbox_insert AFTER INSERT ON mail BEGIN
    UPDATE search_outbox_state SET latest_sequence = latest_sequence + 1 WHERE singleton = 1;
    INSERT INTO search_outbox(sequence, message_identifier)
        SELECT latest_sequence, NEW.identifier FROM search_outbox_state WHERE singleton = 1;
    UPDATE mail SET search_revision = (SELECT latest_sequence FROM search_outbox_state WHERE singleton = 1)
        WHERE sequence = NEW.sequence;
END;

CREATE TRIGGER mail_search_outbox_delete AFTER DELETE ON mail BEGIN
    UPDATE search_outbox_state SET latest_sequence = latest_sequence + 1 WHERE singleton = 1;
    INSERT INTO search_outbox(sequence, message_identifier)
        SELECT latest_sequence, OLD.identifier FROM search_outbox_state WHERE singleton = 1;
END;

-- Revision-only updates do not recurse or enqueue a second operation.
CREATE TRIGGER mail_search_outbox_update AFTER UPDATE OF
    subject, text_body, markup_body, message_identifier, raw_message, received_at, size
ON mail BEGIN
    UPDATE search_outbox_state SET latest_sequence = latest_sequence + 1 WHERE singleton = 1;
    INSERT INTO search_outbox(sequence, message_identifier)
        SELECT latest_sequence, NEW.identifier FROM search_outbox_state WHERE singleton = 1;
    UPDATE mail SET search_revision = (SELECT latest_sequence FROM search_outbox_state WHERE singleton = 1)
        WHERE sequence = NEW.sequence;
END;

-- A revision must identify the newest queued operation for this message; this prevents reuse.
CREATE TRIGGER mail_search_revision_guard BEFORE UPDATE OF search_revision ON mail
WHEN NEW.search_revision <= OLD.search_revision
    OR NEW.search_revision != (SELECT latest_sequence FROM search_outbox_state WHERE singleton = 1)
    OR NOT EXISTS (SELECT 1 FROM search_outbox WHERE sequence = NEW.search_revision AND message_identifier = NEW.identifier)
BEGIN SELECT RAISE(ABORT, 'mail revision must reference a newer search operation'); END;

-- Child mutations reuse the scalar mail update path so normalized changes enqueue refreshes.
CREATE TRIGGER mail_recipients_search_revision_insert AFTER INSERT ON mail_recipients BEGIN
    UPDATE mail SET subject = subject WHERE sequence = NEW.mail_sequence;
END;
CREATE TRIGGER mail_recipients_search_revision_delete AFTER DELETE ON mail_recipients BEGIN
    UPDATE mail SET subject = subject WHERE sequence = OLD.mail_sequence;
END;
CREATE TRIGGER mail_recipients_search_revision_update AFTER UPDATE ON mail_recipients BEGIN
    UPDATE mail SET subject = subject WHERE sequence IN (OLD.mail_sequence, NEW.mail_sequence);
END;

CREATE TRIGGER mail_headers_search_revision_insert AFTER INSERT ON mail_headers BEGIN
    UPDATE mail SET subject = subject WHERE sequence = NEW.mail_sequence;
END;
CREATE TRIGGER mail_headers_search_revision_delete AFTER DELETE ON mail_headers BEGIN
    UPDATE mail SET subject = subject WHERE sequence = OLD.mail_sequence;
END;
CREATE TRIGGER mail_headers_search_revision_update AFTER UPDATE ON mail_headers BEGIN
    UPDATE mail SET subject = subject WHERE sequence IN (OLD.mail_sequence, NEW.mail_sequence);
END;

CREATE TRIGGER mail_attachments_search_revision_insert AFTER INSERT ON mail_attachments BEGIN
    UPDATE mail SET subject = subject WHERE sequence = NEW.mail_sequence;
END;
CREATE TRIGGER mail_attachments_search_revision_delete AFTER DELETE ON mail_attachments BEGIN
    UPDATE mail SET subject = subject WHERE sequence = OLD.mail_sequence;
END;
CREATE TRIGGER mail_attachments_search_revision_update AFTER UPDATE ON mail_attachments BEGIN
    UPDATE mail SET subject = subject WHERE sequence IN (OLD.mail_sequence, NEW.mail_sequence);
END;
