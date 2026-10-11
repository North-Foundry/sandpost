//! Ordered schema operations, grouped into immutable version batches.

/// Current physical schema version, equal to the last migration batch.
pub(crate) const SCHEMA_VERSION: i64 = 1;

/// One checked-in migration script and its file name.
pub(crate) type MigrationScript = (&'static str, &'static str);

/// Version 1 creates every table and trigger group directly in its current form.
const BASELINE_MIGRATIONS: [MigrationScript; 25] = [
    (
        "0001_create_migrations_table.sql",
        include_str!("../../migrations/0001_create_migrations_table.sql"),
    ),
    (
        "0002_create_users_table.sql",
        include_str!("../../migrations/0002_create_users_table.sql"),
    ),
    (
        "0003_create_smtp_server_table.sql",
        include_str!("../../migrations/0003_create_smtp_server_table.sql"),
    ),
    (
        "0004_create_smtp_accesses_table.sql",
        include_str!("../../migrations/0004_create_smtp_accesses_table.sql"),
    ),
    (
        "0005_create_scopes_table.sql",
        include_str!("../../migrations/0005_create_scopes_table.sql"),
    ),
    (
        "0006_create_scope_memberships_table.sql",
        include_str!("../../migrations/0006_create_scope_memberships_table.sql"),
    ),
    (
        "0007_create_views_table.sql",
        include_str!("../../migrations/0007_create_views_table.sql"),
    ),
    (
        "0008_create_mail_table.sql",
        include_str!("../../migrations/0008_create_mail_table.sql"),
    ),
    (
        "0009_create_mail_recipients_table.sql",
        include_str!("../../migrations/0009_create_mail_recipients_table.sql"),
    ),
    (
        "0010_create_mail_headers_table.sql",
        include_str!("../../migrations/0010_create_mail_headers_table.sql"),
    ),
    (
        "0011_create_mail_attachments_table.sql",
        include_str!("../../migrations/0011_create_mail_attachments_table.sql"),
    ),
    (
        "0012_create_search_outbox_state_table.sql",
        include_str!("../../migrations/0012_create_search_outbox_state_table.sql"),
    ),
    (
        "0013_create_search_outbox_table.sql",
        include_str!("../../migrations/0013_create_search_outbox_table.sql"),
    ),
    (
        "0014_create_mail_immutability_triggers.sql",
        include_str!("../../migrations/0014_create_mail_immutability_triggers.sql"),
    ),
    (
        "0015_create_mail_search_outbox_triggers.sql",
        include_str!("../../migrations/0015_create_mail_search_outbox_triggers.sql"),
    ),
    (
        "0016_create_mail_child_fact_triggers.sql",
        include_str!("../../migrations/0016_create_mail_child_fact_triggers.sql"),
    ),
    (
        "0017_create_imap_accounts_table.sql",
        include_str!("../../migrations/0017_create_imap_accounts_table.sql"),
    ),
    (
        "0018_create_imap_account_views_table.sql",
        include_str!("../../migrations/0018_create_imap_account_views_table.sql"),
    ),
    (
        "0019_create_imap_local_folders_table.sql",
        include_str!("../../migrations/0019_create_imap_local_folders_table.sql"),
    ),
    (
        "0020_create_imap_uid_validity_table.sql",
        include_str!("../../migrations/0020_create_imap_uid_validity_table.sql"),
    ),
    (
        "0021_create_imap_mailboxes_table.sql",
        include_str!("../../migrations/0021_create_imap_mailboxes_table.sql"),
    ),
    (
        "0022_create_imap_mailbox_messages_table.sql",
        include_str!("../../migrations/0022_create_imap_mailbox_messages_table.sql"),
    ),
    (
        "0023_create_imap_mailbox_exclusions_table.sql",
        include_str!("../../migrations/0023_create_imap_mailbox_exclusions_table.sql"),
    ),
    (
        "0024_create_imap_message_flags_table.sql",
        include_str!("../../migrations/0024_create_imap_message_flags_table.sql"),
    ),
    (
        "0025_create_imap_unsubscribed_mailboxes_table.sql",
        include_str!("../../migrations/0025_create_imap_unsubscribed_mailboxes_table.sql"),
    ),
];

/// Every migration batch in version order; its number is the schema version it produces.
pub(crate) const MIGRATION_BATCHES: [(i64, &[MigrationScript]); 1] = [(1, &BASELINE_MIGRATIONS)];

/// Return the batches that produce the schema of `version`, in application order.
pub(crate) fn batches_through(
    version: i64,
) -> impl Iterator<Item = &'static (i64, &'static [MigrationScript])> {
    MIGRATION_BATCHES
        .iter()
        .filter(move |(batch, _)| *batch <= version)
}
