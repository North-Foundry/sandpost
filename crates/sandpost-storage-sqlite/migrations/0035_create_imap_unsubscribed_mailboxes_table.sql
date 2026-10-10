-- Mailbox names an IMAP account unsubscribed from; every other name counts as subscribed.
CREATE TABLE imap_unsubscribed_mailboxes (
    account_identifier TEXT NOT NULL REFERENCES imap_accounts(identifier) ON DELETE CASCADE,
    name TEXT NOT NULL,
    PRIMARY KEY (account_identifier, name)
);
