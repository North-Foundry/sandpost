-- Endpoints are logical mail destinations without listeners of their own. The table is rebuilt so
-- the same script normalizes both version 1 layouts (with and without per-endpoint listener
-- columns); identifiers and names are copied unchanged and every reference to them stays valid.
-- The migration runner disables foreign-key enforcement for the rebuild and verifies every
-- reference before committing.
CREATE TABLE endpoints_rebuilt (
    identifier TEXT PRIMARY KEY,
    name TEXT NOT NULL
);

INSERT INTO endpoints_rebuilt(identifier, name)
SELECT identifier, name FROM endpoints;

DROP TABLE endpoints;

ALTER TABLE endpoints_rebuilt RENAME TO endpoints;
