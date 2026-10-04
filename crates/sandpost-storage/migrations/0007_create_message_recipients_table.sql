
CREATE TABLE message_recipients (
    message_seq INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
    address TEXT NOT NULL,
    domain TEXT NOT NULL,
    PRIMARY KEY (message_seq, address)
);

CREATE INDEX message_recipients_domain ON message_recipients(domain, message_seq);
