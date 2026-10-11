-- Materialized IMAP mailbox membership. Each row gives one message its stable UID in one mailbox.
-- Deleting the mail or the mailbox removes the membership; the message itself is never deleted by
-- IMAP. `deleted` is the per-mailbox \Deleted flag and `modification` the account flag counter
-- value of its last change.
CREATE TABLE imap_mailbox_messages (
    mailbox_identifier TEXT NOT NULL REFERENCES imap_mailboxes(identifier) ON DELETE CASCADE,
    uid INTEGER NOT NULL CHECK (uid BETWEEN 1 AND 4294967295),
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
    modification INTEGER NOT NULL DEFAULT 0 CHECK (modification >= 0),
    PRIMARY KEY (mailbox_identifier, uid),
    UNIQUE (mailbox_identifier, mail_sequence)
);

CREATE INDEX imap_mailbox_messages_mail ON imap_mailbox_messages(mail_sequence);
CREATE INDEX imap_mailbox_messages_modification
    ON imap_mailbox_messages(mailbox_identifier, modification);
