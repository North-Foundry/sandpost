-- Durable search-outbox watermarks (no foreign key so deletes stay replayable).
CREATE TABLE search_outbox_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    latest_sequence INTEGER NOT NULL DEFAULT 0 CHECK (latest_sequence >= 0),
    indexed_sequence INTEGER NOT NULL DEFAULT 0 CHECK (indexed_sequence >= 0)
);

INSERT INTO search_outbox_state(singleton) VALUES (1);
