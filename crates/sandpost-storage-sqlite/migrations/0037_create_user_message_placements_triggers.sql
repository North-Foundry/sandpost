-- Require Web placement destinations to belong to the same user.
CREATE TRIGGER user_message_placements_folder_owner_insert
BEFORE INSERT ON user_message_placements
WHEN NEW.folder_identifier IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM web_folders
    WHERE identifier = NEW.folder_identifier AND owner_identifier = NEW.owner_identifier
)
BEGIN SELECT RAISE(ABORT, 'placement folder belongs to another user'); END;

CREATE TRIGGER user_message_placements_folder_owner_update
BEFORE UPDATE ON user_message_placements
WHEN NEW.folder_identifier IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM web_folders
    WHERE identifier = NEW.folder_identifier AND owner_identifier = NEW.owner_identifier
)
BEGIN SELECT RAISE(ABORT, 'placement folder belongs to another user'); END;
