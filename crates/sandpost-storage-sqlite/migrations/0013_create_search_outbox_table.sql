-- Ordered, replayable mail changes awaiting index synchronization.
CREATE TABLE search_outbox (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    message_identifier TEXT NOT NULL
);
