-- One canonical user record: identity, credentials, instance-wide role, and personal filter.
CREATE TABLE users (
    identifier TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    global_role TEXT NOT NULL CHECK (global_role IN ('owner', 'admin', 'member')),
    personal_filter TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
