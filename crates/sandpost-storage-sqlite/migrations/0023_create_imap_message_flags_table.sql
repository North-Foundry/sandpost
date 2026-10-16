-- IMAP flags shared by every mailbox of one account: \Seen, \Answered, \Flagged, \Draft, and
-- keywords (space-separated atoms). A missing row means no flag is set. `modification` is the
-- account flag counter value of the last change.
CREATE TABLE imap_message_flags (
    account_identifier TEXT NOT NULL REFERENCES imap_accounts(identifier) ON DELETE CASCADE,
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    seen INTEGER NOT NULL DEFAULT 0 CHECK (seen IN (0, 1)),
    answered INTEGER NOT NULL DEFAULT 0 CHECK (answered IN (0, 1)),
    flagged INTEGER NOT NULL DEFAULT 0 CHECK (flagged IN (0, 1)),
    draft INTEGER NOT NULL DEFAULT 0 CHECK (draft IN (0, 1)),
    keywords TEXT NOT NULL DEFAULT '',
    modification INTEGER NOT NULL CHECK (modification >= 0),
    PRIMARY KEY (account_identifier, mail_sequence)
);

CREATE INDEX imap_message_flags_mail ON imap_message_flags(mail_sequence);
CREATE INDEX imap_message_flags_modification
    ON imap_message_flags(account_identifier, modification);
