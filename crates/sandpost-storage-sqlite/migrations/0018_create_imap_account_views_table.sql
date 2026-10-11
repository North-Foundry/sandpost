-- Saved Views selected for an IMAP account that does not mirror every View. Deleting the View
-- or the account removes the selection; an optional alias overrides the View's name.
CREATE TABLE imap_account_views (
    account_identifier TEXT NOT NULL REFERENCES imap_accounts(identifier) ON DELETE CASCADE,
    view_identifier TEXT NOT NULL REFERENCES views(identifier) ON DELETE CASCADE,
    alias TEXT,
    PRIMARY KEY (account_identifier, view_identifier)
);

CREATE INDEX imap_account_views_view ON imap_account_views(view_identifier);
