-- Enqueue parent mail inserts, deletes, and searchable updates in the same SQL transaction.
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
    subject, text_body, markup_body, message_identifier, raw_message, received_at, size, endpoint_identifier
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
