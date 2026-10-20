-- Steps of an execution, recorded before each effect starts and updated when it ends.
CREATE TABLE persona_execution_actions (
    execution_identifier TEXT NOT NULL
        REFERENCES persona_executions(identifier) ON DELETE CASCADE,
    step INTEGER NOT NULL CHECK (step >= 0),
    kind TEXT NOT NULL CHECK (kind IN ('branch', 'open', 'wait', 'click')),
    status TEXT NOT NULL
        CHECK (status IN ('started', 'succeeded', 'failed', 'interrupted', 'cancelled')),
    source_line INTEGER NOT NULL CHECK (source_line >= 1),
    source_column INTEGER NOT NULL CHECK (source_column >= 1),
    started_at INTEGER NOT NULL,
    duration_milliseconds INTEGER CHECK (duration_milliseconds IS NULL OR duration_milliseconds >= 0),
    error_message TEXT,
    detail TEXT NOT NULL CHECK (json_valid(detail)),
    PRIMARY KEY (execution_identifier, step)
);
