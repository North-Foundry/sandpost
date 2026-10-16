-- Per-user deletion state keeps shared mail available to other users.
CREATE TABLE user_message_deletions (
    user_identifier TEXT NOT NULL REFERENCES users(identifier) ON DELETE CASCADE,
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    PRIMARY KEY (user_identifier, mail_sequence)
);

CREATE INDEX user_message_deletions_mail ON user_message_deletions(mail_sequence);
