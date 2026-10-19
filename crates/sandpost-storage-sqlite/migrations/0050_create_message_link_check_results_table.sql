-- One verification result per distinct URL of a message's stored link check.
CREATE TABLE message_link_check_results (
    message_identifier TEXT NOT NULL
        REFERENCES message_link_checks(message_identifier) ON DELETE CASCADE,
    url TEXT NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    outcome TEXT NOT NULL
        CHECK (outcome IN ('healthy', 'broken', 'server_error', 'failed', 'inconclusive')),
    status_code INTEGER CHECK (status_code BETWEEN 100 AND 999),
    final_url TEXT,
    redirect_count INTEGER NOT NULL DEFAULT 0 CHECK (redirect_count >= 0),
    error_kind TEXT,
    error_message TEXT,
    checked_at INTEGER NOT NULL,
    PRIMARY KEY (message_identifier, url),
    UNIQUE (message_identifier, position)
);
