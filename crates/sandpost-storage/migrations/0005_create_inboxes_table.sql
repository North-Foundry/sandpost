
CREATE TABLE inboxes (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    filter TEXT NOT NULL
);

CREATE INDEX inboxes_user ON inboxes(user_id, name);
