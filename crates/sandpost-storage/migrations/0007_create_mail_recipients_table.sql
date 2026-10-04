
CREATE TABLE mail_recipients (
    mail_sequence INTEGER NOT NULL REFERENCES mail(sequence) ON DELETE CASCADE,
    recipient_type TEXT NOT NULL CHECK (recipient_type IN ('envelope_from', 'envelope_to', 'from', 'to', 'carbon_copy')),
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
    address TEXT NOT NULL,
    domain TEXT NOT NULL,
    PRIMARY KEY (mail_sequence, recipient_type, ordinal),
    CHECK (recipient_type != 'envelope_from' OR ordinal = 0)
);

CREATE INDEX mail_recipients_address ON mail_recipients(recipient_type, address, mail_sequence);
CREATE INDEX mail_recipients_domain ON mail_recipients(recipient_type, domain, mail_sequence);
