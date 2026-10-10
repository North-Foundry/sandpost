-- Applied only when upgrading the per-endpoint listener variant of version 1, after
-- 0017_create_smtp_server_table.sql and before endpoint listener columns are dropped.
--
-- The global SMTP server inherits the default endpoint's listener, or else the first endpoint (by
-- identifier) that had one. Without any listener the server stays disabled, as it was before.
UPDATE smtp_server
SET smtp_host = (
        SELECT smtp_host FROM endpoints WHERE smtp_host IS NOT NULL
        ORDER BY identifier <> '00000000-0000-0000-0000-000000000002', identifier LIMIT 1
    ),
    smtp_port = (
        SELECT smtp_port FROM endpoints WHERE smtp_host IS NOT NULL
        ORDER BY identifier <> '00000000-0000-0000-0000-000000000002', identifier LIMIT 1
    )
WHERE singleton = 1;
