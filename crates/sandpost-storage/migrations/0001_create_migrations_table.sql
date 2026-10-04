
CREATE TABLE migrations (
    migration TEXT PRIMARY KEY NOT NULL,
    batch INTEGER NOT NULL CHECK(batch > 0)
);
