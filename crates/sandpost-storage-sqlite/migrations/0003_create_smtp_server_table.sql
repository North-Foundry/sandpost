-- Global SMTP listener, initially enabled at 127.0.0.1:1025.
CREATE TABLE smtp_server (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    smtp_host TEXT,
    smtp_port INTEGER CHECK (smtp_port BETWEEN 0 AND 65535),
    CHECK ((smtp_host IS NULL) = (smtp_port IS NULL))
);

INSERT INTO smtp_server(singleton, smtp_host, smtp_port) VALUES (1, '127.0.0.1', 1025);
