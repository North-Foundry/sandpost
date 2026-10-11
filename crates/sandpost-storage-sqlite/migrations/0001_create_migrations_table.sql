-- Applied schema operations and their version batch.
CREATE TABLE migrations (
    migration TEXT PRIMARY KEY NOT NULL,
    batch INTEGER NOT NULL CHECK(batch > 0)
);
