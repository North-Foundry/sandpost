-- Schema version 3: endpoints are removed and all captured mail forms one pool.
--
-- Users gain the read-only `viewer` global role and an instance-wide mail access mode that
-- replaces per-endpoint memberships. The conversion never widens visibility:
-- - owners keep reading all mail;
-- - any other account reads all mail only if it had `all` mail access on every endpoint,
--   otherwise it becomes `scoped` and keeps its scope assignments;
-- - a member whose endpoint roles were all `viewer` becomes a global viewer. Endpoint
--   administrators keep their global role (scope administration now needs a global admin).
CREATE TABLE users_rebuilt (
    identifier TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    global_role TEXT NOT NULL CHECK (global_role IN ('owner', 'admin', 'member', 'viewer')),
    mail_access TEXT NOT NULL CHECK (mail_access IN ('all', 'scoped')),
    personal_filter TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT INTO users_rebuilt(
    identifier, name, email, password_hash, global_role, mail_access, personal_filter, created_at,
    updated_at
)
SELECT
    account.identifier,
    account.name,
    account.email,
    account.password_hash,
    CASE
        WHEN account.global_role = 'member'
            AND EXISTS (
                SELECT 1 FROM endpoint_memberships AS membership
                WHERE membership.user_identifier = account.identifier
            )
            AND NOT EXISTS (
                SELECT 1 FROM endpoint_memberships AS membership
                WHERE membership.user_identifier = account.identifier
                    AND membership.role != 'viewer'
            )
        THEN 'viewer'
        ELSE account.global_role
    END,
    CASE
        WHEN account.global_role = 'owner' THEN 'all'
        WHEN NOT EXISTS (
            SELECT 1 FROM endpoints AS endpoint
            WHERE NOT EXISTS (
                SELECT 1 FROM endpoint_memberships AS membership
                WHERE membership.user_identifier = account.identifier
                    AND membership.endpoint_identifier = endpoint.identifier
                    AND membership.mail_access = 'all'
            )
        ) THEN 'all'
        ELSE 'scoped'
    END,
    account.personal_filter,
    account.created_at,
    account.updated_at
FROM users AS account;

DROP TABLE users;

ALTER TABLE users_rebuilt RENAME TO users;
