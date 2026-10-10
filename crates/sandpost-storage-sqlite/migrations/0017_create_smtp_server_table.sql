-- Schema version 2: one global SMTP server receives the mail of every endpoint.
--
-- The singleton row persists the server's listener address; host and port are null together when
-- the server is disabled. It starts at 127.0.0.1:1025, the SMTP address version 1 used by default.
-- Installations created with per-endpoint listeners instead inherit their primary listener address
-- (see variants/0017_copy_endpoint_listener_address.sql).
CREATE TABLE smtp_server (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    smtp_host TEXT,
    smtp_port INTEGER CHECK (smtp_port BETWEEN 0 AND 65535),
    CHECK ((smtp_host IS NULL) = (smtp_port IS NULL))
);

INSERT INTO smtp_server(singleton, smtp_host, smtp_port) VALUES (1, '127.0.0.1', 1025);
