-- Preserve existing Trash memberships as durable account-local copies.
INSERT INTO imap_message_copies(account_identifier, mail_sequence, target_kind)
SELECT box.account_identifier, member.mail_sequence, 'trash'
FROM imap_mailbox_messages AS member
JOIN imap_mailboxes AS box ON box.identifier = member.mailbox_identifier
WHERE box.source = 'trash';
