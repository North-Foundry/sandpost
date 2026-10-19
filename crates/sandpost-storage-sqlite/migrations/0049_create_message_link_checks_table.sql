-- The most recent completed link check of each physical message, shared by its readers.
CREATE TABLE message_link_checks (
    message_identifier TEXT PRIMARY KEY REFERENCES mail(identifier) ON DELETE CASCADE,
    checked_at INTEGER NOT NULL,
    omitted_link_count INTEGER NOT NULL DEFAULT 0 CHECK (omitted_link_count >= 0)
);
