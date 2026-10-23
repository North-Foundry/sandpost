-- Attempts of a delivery job, recorded when they start and completed when they end.
CREATE TABLE outbound_delivery_attempts (
    delivery_identifier TEXT NOT NULL REFERENCES outbound_deliveries(identifier) ON DELETE CASCADE,
    number INTEGER NOT NULL CHECK (number >= 1),
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    outcome TEXT CHECK (outcome IS NULL OR outcome IN ('delivered', 'partial', 'deferred', 'failed', 'interrupted')),
    error_kind TEXT,
    error_message TEXT,
    error_code INTEGER,
    delivered_recipients INTEGER NOT NULL DEFAULT 0 CHECK (delivered_recipients >= 0),
    deferred_recipients INTEGER NOT NULL DEFAULT 0 CHECK (deferred_recipients >= 0),
    failed_recipients INTEGER NOT NULL DEFAULT 0 CHECK (failed_recipients >= 0),
    PRIMARY KEY (delivery_identifier, number)
);
