-- SandPost SQLite schema baseline, one operation per file.
--
-- There is no incremental migration history. SandPost has not been deployed, so a fresh database
-- is created directly at this schema and any other user_version is rejected rather than migrated.
--
-- Logical model (backend-independent): users with a global role; endpoints; endpoint memberships
-- (authority role plus independent mail-access mode); hierarchical scopes; role-less scope
-- memberships; personal/shared views; authoritative mail with normalized child facts; and the
-- durable search outbox. The physical layout is a SQLite implementation detail.

CREATE TABLE migrations (
    migration TEXT PRIMARY KEY NOT NULL,
    batch INTEGER NOT NULL CHECK(batch > 0)
);
