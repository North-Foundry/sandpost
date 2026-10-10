-- Persistent identities of IMAP mailboxes. A mailbox's source is the account INBOX, its Trash, a
-- linked View, or a local folder. UIDs are assigned from `uid_next` and never reused.
-- `examined_sequence` is the highest mail sequence already evaluated against
-- `definition_fingerprint`; a different fingerprint forces a full reconciliation.
-- `recent_through` is the highest UID already reported as \Recent to a read-write session.
CREATE TABLE imap_mailboxes (
    identifier TEXT PRIMARY KEY,
    account_identifier TEXT NOT NULL REFERENCES imap_accounts(identifier) ON DELETE CASCADE,
    source TEXT NOT NULL CHECK (source IN ('inbox', 'trash', 'view', 'folder')),
    view_identifier TEXT REFERENCES views(identifier) ON DELETE CASCADE,
    folder_identifier TEXT REFERENCES imap_local_folders(identifier) ON DELETE CASCADE,
    uid_validity INTEGER NOT NULL CHECK (uid_validity BETWEEN 1 AND 4294967295),
    uid_next INTEGER NOT NULL DEFAULT 1 CHECK (uid_next BETWEEN 1 AND 4294967296),
    recent_through INTEGER NOT NULL DEFAULT 0 CHECK (recent_through >= 0),
    examined_sequence INTEGER NOT NULL DEFAULT 0 CHECK (examined_sequence >= 0),
    definition_fingerprint TEXT,
    CHECK ((source = 'view') = (view_identifier IS NOT NULL)),
    CHECK ((source = 'folder') = (folder_identifier IS NOT NULL))
);

CREATE UNIQUE INDEX imap_mailboxes_fixed_source ON imap_mailboxes(account_identifier, source)
    WHERE source IN ('inbox', 'trash');
CREATE UNIQUE INDEX imap_mailboxes_view ON imap_mailboxes(account_identifier, view_identifier)
    WHERE view_identifier IS NOT NULL;
CREATE UNIQUE INDEX imap_mailboxes_folder ON imap_mailboxes(folder_identifier)
    WHERE folder_identifier IS NOT NULL;
CREATE INDEX imap_mailboxes_view_reference ON imap_mailboxes(view_identifier);
