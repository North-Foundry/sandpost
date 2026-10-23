-- Reusable connections to external SMTP servers. The password is stored only encrypted with a
-- key kept outside the database; recipient patterns are a JSON array of strings.
CREATE TABLE smtp_outbounds (
    identifier TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE CHECK (name <> ''),
    host TEXT NOT NULL CHECK (host <> ''),
    port INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    security TEXT NOT NULL CHECK (security IN ('none', 'starttls', 'tls')),
    authentication TEXT NOT NULL CHECK (authentication IN ('none', 'plain', 'login')),
    username TEXT,
    encrypted_password BLOB,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    override_from TEXT,
    envelope_from TEXT,
    connect_timeout_seconds INTEGER CHECK (connect_timeout_seconds IS NULL OR connect_timeout_seconds >= 1),
    smtp_timeout_seconds INTEGER CHECK (smtp_timeout_seconds IS NULL OR smtp_timeout_seconds >= 1),
    verify_certificates INTEGER NOT NULL CHECK (verify_certificates IN (0, 1)),
    helo_name TEXT,
    maximum_connections INTEGER CHECK (maximum_connections IS NULL OR maximum_connections >= 1),
    allowed_recipients TEXT NOT NULL CHECK (json_valid(allowed_recipients) AND json_type(allowed_recipients) = 'array'),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
