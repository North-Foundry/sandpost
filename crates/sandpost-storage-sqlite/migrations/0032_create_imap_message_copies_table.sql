-- Additional account-local message occurrences and their lookup indexes.
CREATE TABLE imap_message_copies (
    account_identifier TEXT NOT NULL REFERENCES imap_accounts(identifier) ON DELETE CASCADE,
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('inbox', 'trash', 'folder')),
    folder_identifier TEXT REFERENCES imap_local_folders(identifier) ON DELETE CASCADE,
    CHECK ((target_kind = 'folder') = (folder_identifier IS NOT NULL))
);

CREATE UNIQUE INDEX imap_message_copies_folder
    ON imap_message_copies(account_identifier, mail_sequence, folder_identifier)
    WHERE target_kind = 'folder';

CREATE UNIQUE INDEX imap_message_copies_general
    ON imap_message_copies(account_identifier, mail_sequence, target_kind)
    WHERE target_kind != 'folder';

CREATE INDEX imap_message_copies_folder_lookup ON imap_message_copies(folder_identifier);
