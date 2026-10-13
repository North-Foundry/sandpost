-- Per-user deletion state keeps shared mail available to other users.
CREATE TABLE user_message_deletions (
    user_identifier TEXT NOT NULL REFERENCES users(identifier) ON DELETE CASCADE,
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    PRIMARY KEY (user_identifier, mail_sequence)
);

CREATE INDEX user_message_deletions_mail ON user_message_deletions(mail_sequence);

-- A user deletion removes any materialized IMAP memberships owned by that user.
CREATE TRIGGER user_message_deletions_remove_imap_memberships
AFTER INSERT ON user_message_deletions
BEGIN
    DELETE FROM imap_mailbox_messages
    WHERE mail_sequence = NEW.mail_sequence
      AND mailbox_identifier IN (
          SELECT imap_mailboxes.identifier
          FROM imap_mailboxes
          JOIN imap_accounts
            ON imap_accounts.identifier = imap_mailboxes.account_identifier
          WHERE imap_accounts.owner_identifier = NEW.user_identifier
      );
END;
