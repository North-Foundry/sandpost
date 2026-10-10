-- Schema version 4: each SMTP access chooses whether it requires TLS and which AUTH mechanisms it
-- accepts. Existing accesses keep accepting PLAIN and LOGIN on any connection. At least one
-- mechanism stays allowed; the storage operations enforce that, not a table constraint.
ALTER TABLE smtp_accesses
    ADD COLUMN requires_encryption INTEGER NOT NULL DEFAULT 0 CHECK (requires_encryption IN (0, 1));

ALTER TABLE smtp_accesses
    ADD COLUMN plain_mechanism_allowed INTEGER NOT NULL DEFAULT 1
        CHECK (plain_mechanism_allowed IN (0, 1));

ALTER TABLE smtp_accesses
    ADD COLUMN login_mechanism_allowed INTEGER NOT NULL DEFAULT 1
        CHECK (login_mechanism_allowed IN (0, 1));
