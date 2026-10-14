DROP TRIGGER user_message_deletions_remove_imap_memberships;
CREATE TRIGGER user_message_deletions_remove_imap_memberships AFTER INSERT ON user_message_deletions
BEGIN
    DELETE FROM imap_mailbox_messages WHERE mail_sequence = NEW.mail_sequence
      AND mailbox_identifier IN (SELECT box.identifier FROM imap_mailboxes AS box
      JOIN imap_accounts AS account ON account.identifier = box.account_identifier
      WHERE account.owner_identifier = NEW.user_identifier AND box.organization_mode = 'mirror');
    DELETE FROM user_message_placements WHERE owner_identifier = NEW.user_identifier AND mail_sequence = NEW.mail_sequence;
    DELETE FROM user_message_copies WHERE owner_identifier = NEW.user_identifier AND mail_sequence = NEW.mail_sequence;
END;
