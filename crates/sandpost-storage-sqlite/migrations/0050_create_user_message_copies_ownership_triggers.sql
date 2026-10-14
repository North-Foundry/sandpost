CREATE TRIGGER user_message_copies_folder_owner_insert BEFORE INSERT ON user_message_copies
WHEN NEW.folder_identifier IS NOT NULL AND NOT EXISTS(SELECT 1 FROM web_folders
    WHERE identifier = NEW.folder_identifier AND owner_identifier = NEW.owner_identifier)
BEGIN SELECT RAISE(ABORT, 'copy folder belongs to another user'); END;

CREATE TRIGGER user_message_copies_folder_owner_update BEFORE UPDATE ON user_message_copies
WHEN NEW.folder_identifier IS NOT NULL AND NOT EXISTS(SELECT 1 FROM web_folders
    WHERE identifier = NEW.folder_identifier AND owner_identifier = NEW.owner_identifier)
BEGIN SELECT RAISE(ABORT, 'copy folder belongs to another user'); END;
