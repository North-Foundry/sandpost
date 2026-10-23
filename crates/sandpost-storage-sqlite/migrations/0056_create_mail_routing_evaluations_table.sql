-- Stored messages waiting for routing evaluation, queued in the message's insert transaction
-- while any enabled routing action uses an enabled SMTP Outbound.
CREATE TABLE mail_routing_evaluations (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    message_identifier TEXT NOT NULL UNIQUE REFERENCES mail(identifier) ON DELETE CASCADE,
    queued_at INTEGER NOT NULL
);
