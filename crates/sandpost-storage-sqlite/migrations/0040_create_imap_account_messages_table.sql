-- Durable independent retention exists even while a credential exposes Mirror.
CREATE TABLE imap_account_messages (
    account_identifier TEXT NOT NULL REFERENCES imap_accounts(identifier) ON DELETE CASCADE,
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    PRIMARY KEY(account_identifier, mail_sequence)
);
CREATE INDEX imap_account_messages_mail ON imap_account_messages(mail_sequence);
