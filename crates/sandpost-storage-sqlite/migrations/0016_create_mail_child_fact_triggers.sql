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
