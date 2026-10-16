-- Immutable mail identity and ordering keys.
CREATE TRIGGER mail_identifier_immutable BEFORE UPDATE OF identifier ON mail
WHEN NEW.identifier != OLD.identifier BEGIN SELECT RAISE(ABORT, 'mail identifier is immutable'); END;

CREATE TRIGGER mail_sequence_immutable BEFORE UPDATE OF sequence ON mail
WHEN NEW.sequence != OLD.sequence BEGIN SELECT RAISE(ABORT, 'mail sequence is immutable'); END;
