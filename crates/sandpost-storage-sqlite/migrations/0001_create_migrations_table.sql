-- Applied schema operations and the execution batch that committed them.
CREATE TABLE migrations (
    migration TEXT PRIMARY KEY NOT NULL,
    batch INTEGER NOT NULL DEFAULT 1 CHECK (typeof(batch) = 'integer' AND batch > 0)
);
