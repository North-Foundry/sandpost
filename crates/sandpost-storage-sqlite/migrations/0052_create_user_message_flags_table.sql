CREATE TABLE user_message_flags (
    owner_identifier TEXT NOT NULL REFERENCES users(identifier) ON DELETE CASCADE,
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    seen INTEGER NOT NULL DEFAULT 0 CHECK(seen IN (0,1)),
    answered INTEGER NOT NULL DEFAULT 0 CHECK(answered IN (0,1)),
    flagged INTEGER NOT NULL DEFAULT 0 CHECK(flagged IN (0,1)),
    draft INTEGER NOT NULL DEFAULT 0 CHECK(draft IN (0,1)),
    keywords TEXT NOT NULL DEFAULT '',
    modification INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(owner_identifier, mail_sequence)
);
