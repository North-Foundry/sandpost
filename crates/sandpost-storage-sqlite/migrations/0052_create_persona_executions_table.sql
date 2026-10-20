-- One run of a Persona's script for one stored message, scheduled when the message is stored.
CREATE TABLE persona_executions (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    identifier TEXT NOT NULL UNIQUE,
    persona_identifier TEXT NOT NULL REFERENCES personas(identifier) ON DELETE CASCADE,
    message_identifier TEXT NOT NULL REFERENCES mail(identifier) ON DELETE CASCADE,
    status TEXT NOT NULL
        CHECK (status IN ('pending', 'running', 'completed', 'failed', 'cancelled')),
    scheduled_at INTEGER NOT NULL,
    started_at INTEGER,
    completed_at INTEGER,
    persona_revision INTEGER CHECK (persona_revision IS NULL OR persona_revision >= 1),
    error_message TEXT,
    UNIQUE (persona_identifier, message_identifier)
);

CREATE INDEX persona_executions_status ON persona_executions(status, sequence);
CREATE INDEX persona_executions_persona ON persona_executions(persona_identifier, sequence);
CREATE INDEX persona_executions_message ON persona_executions(message_identifier);
