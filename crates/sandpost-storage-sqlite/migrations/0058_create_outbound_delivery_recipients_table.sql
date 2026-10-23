-- The envelope recipients of a delivery job, each with its own outcome.
CREATE TABLE outbound_delivery_recipients (
    delivery_identifier TEXT NOT NULL REFERENCES outbound_deliveries(identifier) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    address TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'delivered', 'failed')),
    error_kind TEXT,
    error_message TEXT,
    error_code INTEGER,
    response TEXT,
    delivered_at INTEGER,
    PRIMARY KEY (delivery_identifier, position),
    UNIQUE (delivery_identifier, address)
);
