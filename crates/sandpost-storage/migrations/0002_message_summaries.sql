ALTER TABLE messages ADD COLUMN subject TEXT NOT NULL DEFAULT '';
ALTER TABLE messages ADD COLUMN from_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE messages ADD COLUMN to_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE messages ADD COLUMN size INTEGER NOT NULL DEFAULT 0;
ALTER TABLE messages ADD COLUMN attachment_count INTEGER NOT NULL DEFAULT 0;

UPDATE messages SET
    subject = COALESCE(json_extract(facts, '$.subject'), ''),
    from_json = COALESCE(json_extract(facts, '$.from'), '[]'),
    to_json = COALESCE(json_extract(facts, '$.to'), '[]'),
    size = CAST(COALESCE(json_extract(facts, '$.size'), 0) AS INTEGER),
    attachment_count = CAST(COALESCE(json_extract(facts, '$.attachment_count'), 0) AS INTEGER);
