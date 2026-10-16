CREATE TRIGGER mail_seed_independent_references AFTER INSERT ON mail
BEGIN
    INSERT INTO imap_account_messages(account_identifier, mail_sequence)
    SELECT identifier, NEW.sequence FROM imap_accounts;
END;

