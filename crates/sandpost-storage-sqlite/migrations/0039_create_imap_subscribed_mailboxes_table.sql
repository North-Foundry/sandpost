-- Retain subscribed names independently of the current mailbox catalog.
CREATE TABLE imap_subscribed_mailboxes (
    account_identifier TEXT NOT NULL REFERENCES imap_accounts(identifier) ON DELETE CASCADE,
    name TEXT NOT NULL,
    PRIMARY KEY (account_identifier, name)
);
