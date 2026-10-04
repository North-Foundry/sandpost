
CREATE TABLE inboxes (
    identifier TEXT PRIMARY KEY,
    user_identifier TEXT NOT NULL REFERENCES users(identifier) ON DELETE CASCADE,
    name TEXT NOT NULL,
    filter TEXT NOT NULL
);

CREATE INDEX inboxes_user ON inboxes(user_identifier, name);
