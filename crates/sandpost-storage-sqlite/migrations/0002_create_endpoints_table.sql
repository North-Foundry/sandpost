-- SMTP endpoints own scopes, views, and mail. Seed the default endpoint for endpointless data.
CREATE TABLE endpoints (
    identifier TEXT PRIMARY KEY,
    name TEXT NOT NULL
);

INSERT INTO endpoints(identifier, name)
VALUES ('00000000-0000-0000-0000-000000000002', 'Default endpoint');
