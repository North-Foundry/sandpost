-- Old fingerprints describe placement-dependent smart policy and are not valid after the view split.
UPDATE imap_mailboxes SET definition_fingerprint = NULL;
