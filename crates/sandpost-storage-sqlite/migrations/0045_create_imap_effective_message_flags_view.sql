CREATE VIEW imap_effective_message_flags AS
SELECT flags.* FROM imap_message_flags AS flags JOIN imap_accounts AS account
ON account.identifier = flags.account_identifier WHERE account.mode = 'independent'
UNION ALL
SELECT account.identifier AS account_identifier, flags.mail_sequence, flags.seen,
       flags.answered, flags.flagged, flags.draft, flags.keywords, flags.modification
FROM user_message_flags AS flags JOIN imap_accounts AS account
ON account.owner_identifier = flags.owner_identifier WHERE account.mode = 'mirror';
