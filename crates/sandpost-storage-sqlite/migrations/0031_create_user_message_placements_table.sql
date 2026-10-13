-- Per-user organization of shared messages into Web folders.
CREATE TABLE user_message_placements (
    owner_identifier TEXT NOT NULL REFERENCES users(identifier) ON DELETE CASCADE,
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    folder_identifier TEXT REFERENCES web_folders(identifier) ON DELETE CASCADE,
    PRIMARY KEY (owner_identifier, mail_sequence)
);

CREATE INDEX user_message_placements_folder
    ON user_message_placements(owner_identifier, folder_identifier, mail_sequence);
