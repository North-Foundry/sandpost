-- Per-user Web Trash: messages moved out of every Web mailbox but not yet permanently deleted.
CREATE TABLE user_message_trash (
    owner_identifier TEXT NOT NULL REFERENCES users(identifier) ON DELETE CASCADE,
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    trashed_at INTEGER NOT NULL,
    PRIMARY KEY (owner_identifier, mail_sequence)
);

CREATE INDEX user_message_trash_mail ON user_message_trash(mail_sequence);
