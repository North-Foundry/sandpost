-- One delivery job per message and routing action. References become null when the scope,
-- action or SMTP Outbound is deleted; the recorded type and outbound name remain.
CREATE TABLE outbound_deliveries (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    identifier TEXT NOT NULL UNIQUE,
    message_identifier TEXT NOT NULL REFERENCES mail(identifier) ON DELETE CASCADE,
    scope_identifier TEXT REFERENCES scopes(identifier) ON DELETE SET NULL,
    routing_action_identifier TEXT REFERENCES scope_routing_actions(identifier) ON DELETE SET NULL,
    smtp_outbound_identifier TEXT REFERENCES smtp_outbounds(identifier) ON DELETE SET NULL,
    smtp_outbound_name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('relay', 'forward')),
    status TEXT NOT NULL
        CHECK (status IN ('pending', 'processing', 'retrying', 'delivered', 'failed')),
    envelope_from TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    error_kind TEXT,
    error_message TEXT,
    error_code INTEGER,
    created_at INTEGER NOT NULL,
    last_attempt_at INTEGER,
    next_attempt_at INTEGER,
    delivered_at INTEGER,
    UNIQUE (message_identifier, routing_action_identifier)
);

CREATE INDEX outbound_deliveries_due ON outbound_deliveries(status, next_attempt_at, sequence);
CREATE INDEX outbound_deliveries_message ON outbound_deliveries(message_identifier, sequence);
CREATE INDEX outbound_deliveries_routing_action ON outbound_deliveries(routing_action_identifier);
CREATE INDEX outbound_deliveries_smtp_outbound ON outbound_deliveries(smtp_outbound_identifier);
