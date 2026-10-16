CREATE TABLE user_message_copies (
    owner_identifier TEXT NOT NULL REFERENCES users(identifier) ON DELETE CASCADE,
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    target_kind TEXT NOT NULL CHECK(target_kind IN ('inbox', 'trash', 'folder')),
    folder_identifier TEXT REFERENCES web_folders(identifier) ON DELETE CASCADE,
    CHECK ((target_kind = 'folder') = (folder_identifier IS NOT NULL))
);
CREATE UNIQUE INDEX user_message_copies_folder
ON user_message_copies(owner_identifier, mail_sequence, folder_identifier) WHERE target_kind = 'folder';
CREATE UNIQUE INDEX user_message_copies_general
ON user_message_copies(owner_identifier, mail_sequence, target_kind) WHERE target_kind != 'folder';
