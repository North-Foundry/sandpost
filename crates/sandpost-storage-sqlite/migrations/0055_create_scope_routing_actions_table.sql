-- Relay and forward rules attached to scopes. They disappear with their scope, and an SMTP
-- Outbound cannot be deleted while an action references it.
CREATE TABLE scope_routing_actions (
    identifier TEXT PRIMARY KEY,
    scope_identifier TEXT NOT NULL REFERENCES scopes(identifier) ON DELETE CASCADE,
    smtp_outbound_identifier TEXT NOT NULL REFERENCES smtp_outbounds(identifier) ON DELETE RESTRICT,
    kind TEXT NOT NULL CHECK (kind IN ('relay', 'forward')),
    forward_recipients TEXT NOT NULL CHECK (json_valid(forward_recipients) AND json_type(forward_recipients) = 'array'),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX scope_routing_actions_scope ON scope_routing_actions(scope_identifier, created_at, identifier);
CREATE INDEX scope_routing_actions_outbound ON scope_routing_actions(smtp_outbound_identifier);
