-- Schema version 1 variant written by pre-release builds with per-endpoint SMTP listeners.
--
-- It is never applied. The migration runner compares a version 1 database against it, recognizes
-- such installations, and upgrades them to version 2 like the canonical baseline after copying
-- their primary listener address to the global SMTP server. The statements below must stay exactly
-- as those builds created them.
-- SMTP endpoints own scopes, views, and mail. Seed the default endpoint for endpointless data.
-- smtp_host and smtp_port persist the endpoint's SMTP listener; both are null for an endpoint
-- that has no listener of its own.
CREATE TABLE endpoints (
    identifier TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    smtp_host TEXT,
    smtp_port INTEGER CHECK (smtp_port BETWEEN 0 AND 65535),
    CHECK ((smtp_host IS NULL) = (smtp_port IS NULL))
);

INSERT INTO endpoints(identifier, name, smtp_host, smtp_port)
VALUES ('00000000-0000-0000-0000-000000000002', 'Default endpoint', '127.0.0.1', 1025);
