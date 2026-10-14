-- Permanent user deletion supersedes the user's Web Trash state.
CREATE TRIGGER user_message_deletions_remove_trash
AFTER INSERT ON user_message_deletions
BEGIN
    DELETE FROM user_message_trash
    WHERE owner_identifier = NEW.user_identifier AND mail_sequence = NEW.mail_sequence;
END;
