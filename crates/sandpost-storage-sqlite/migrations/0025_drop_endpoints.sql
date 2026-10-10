-- Endpoint memberships were converted into global roles and user mail access by 0020; nothing
-- references endpoints any more.
DROP TABLE endpoint_memberships;

DROP TABLE endpoints;
