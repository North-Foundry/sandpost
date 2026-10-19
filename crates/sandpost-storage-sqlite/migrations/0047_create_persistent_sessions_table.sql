CREATE TABLE persistent_sessions (
    token_hash TEXT PRIMARY KEY,
    user_identifier TEXT NOT NULL REFERENCES users(identifier) ON DELETE CASCADE,
    verified_password_hash TEXT NOT NULL
);
CREATE INDEX persistent_sessions_user_identifier ON persistent_sessions(user_identifier);
