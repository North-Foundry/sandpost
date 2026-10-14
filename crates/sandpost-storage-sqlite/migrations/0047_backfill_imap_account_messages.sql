INSERT INTO imap_account_messages(account_identifier, mail_sequence)
SELECT account.identifier, mail.sequence FROM imap_accounts AS account CROSS JOIN mail
WHERE NOT EXISTS(SELECT 1 FROM user_message_deletions AS deletion
    WHERE deletion.user_identifier = account.owner_identifier AND deletion.mail_sequence = mail.sequence);
