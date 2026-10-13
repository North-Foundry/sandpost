-- Account-local placement overrides for shared messages.
CREATE TABLE imap_message_placements (
    account_identifier TEXT NOT NULL REFERENCES imap_accounts(identifier) ON DELETE CASCADE,
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    target_kind TEXT NOT NULL DEFAULT 'folder' CHECK (target_kind IN ('inbox', 'trash', 'folder')),
    folder_identifier TEXT REFERENCES imap_local_folders(identifier) ON DELETE CASCADE,
    PRIMARY KEY (account_identifier, mail_sequence),
    CHECK ((target_kind = 'folder') = (folder_identifier IS NOT NULL))
);

CREATE INDEX imap_message_placements_folder
    ON imap_message_placements(account_identifier, folder_identifier, mail_sequence);
