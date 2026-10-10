-- Messages expunged from a dynamic IMAP mailbox. The mailbox filter does not bring them back until
-- the owner restores hidden messages or a client copies them into the mailbox again.
CREATE TABLE imap_mailbox_exclusions (
    mailbox_identifier TEXT NOT NULL REFERENCES imap_mailboxes(identifier) ON DELETE CASCADE,
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    PRIMARY KEY (mailbox_identifier, mail_sequence)
);

CREATE INDEX imap_mailbox_exclusions_mail ON imap_mailbox_exclusions(mail_sequence);
