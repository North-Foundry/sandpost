//! Immutable schema versions, recognized layouts, and ordered migration scripts.

/// Current physical schema version, equal to the number of the last migration batch.
pub(crate) const SCHEMA_VERSION: i64 = 5;

/// One checked-in migration script and its file name.
pub(crate) type MigrationScript = (&'static str, &'static str);

/// The ordered, checked-in scripts that create the complete version 1 schema.
///
/// Each file holds one schema operation: a table with its indexes, or one trigger group. They are
/// applied in filename order as a single baseline and recorded together in the migration history.
const BASELINE_MIGRATIONS: [MigrationScript; 16] = [
    (
        "0001_create_migrations_table.sql",
        include_str!("../../migrations/0001_create_migrations_table.sql"),
    ),
    (
        "0002_create_endpoints_table.sql",
        include_str!("../../migrations/0002_create_endpoints_table.sql"),
    ),
    (
        "0003_create_users_table.sql",
        include_str!("../../migrations/0003_create_users_table.sql"),
    ),
    (
        "0004_create_scopes_table.sql",
        include_str!("../../migrations/0004_create_scopes_table.sql"),
    ),
    (
        "0005_create_scope_memberships_table.sql",
        include_str!("../../migrations/0005_create_scope_memberships_table.sql"),
    ),
    (
        "0006_create_endpoint_memberships_table.sql",
        include_str!("../../migrations/0006_create_endpoint_memberships_table.sql"),
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
];

/// Version 2: one global SMTP server, logical endpoints, and hashed SMTP access credentials.
const SMTP_ACCESS_MIGRATIONS: [MigrationScript; 3] = [
    (
        "0017_create_smtp_server_table.sql",
        include_str!("../../migrations/0017_create_smtp_server_table.sql"),
    ),
    (
        "0018_remove_endpoint_listener_columns.sql",
        include_str!("../../migrations/0018_remove_endpoint_listener_columns.sql"),
    ),
    (
        "0019_create_smtp_accesses_table.sql",
        include_str!("../../migrations/0019_create_smtp_accesses_table.sql"),
    ),
];

/// Version 3: endpoints are removed; mail forms one pool and users carry global mail access.
const SINGLE_MAIL_POOL_MIGRATIONS: [MigrationScript; 6] = [
    (
        "0020_rebuild_users_with_mail_access.sql",
        include_str!("../../migrations/0020_rebuild_users_with_mail_access.sql"),
    ),
    (
        "0021_rebuild_scopes_without_endpoints.sql",
        include_str!("../../migrations/0021_rebuild_scopes_without_endpoints.sql"),
    ),
    (
        "0022_rebuild_views_without_endpoints.sql",
        include_str!("../../migrations/0022_rebuild_views_without_endpoints.sql"),
    ),
    (
        "0023_rebuild_smtp_accesses_without_endpoints.sql",
        include_str!("../../migrations/0023_rebuild_smtp_accesses_without_endpoints.sql"),
    ),
    (
        "0024_rebuild_mail_without_endpoints.sql",
        include_str!("../../migrations/0024_rebuild_mail_without_endpoints.sql"),
    ),
    (
        "0025_drop_endpoints.sql",
        include_str!("../../migrations/0025_drop_endpoints.sql"),
    ),
];

/// Version 4: SMTP accesses choose whether they require TLS and which AUTH mechanisms they accept.
const SMTP_ACCESS_AUTHENTICATION_RULES_MIGRATIONS: [MigrationScript; 1] = [(
    "0026_add_smtp_access_authentication_rules.sql",
    include_str!("../../migrations/0026_add_smtp_access_authentication_rules.sql"),
)];

/// Version 5: user-owned IMAP accounts, their mailbox identities, membership, and flags.
const IMAP_MIGRATIONS: [MigrationScript; 9] = [
    (
        "0027_create_imap_accounts_table.sql",
        include_str!("../../migrations/0027_create_imap_accounts_table.sql"),
    ),
    (
        "0028_create_imap_account_views_table.sql",
        include_str!("../../migrations/0028_create_imap_account_views_table.sql"),
    ),
    (
        "0029_create_imap_local_folders_table.sql",
        include_str!("../../migrations/0029_create_imap_local_folders_table.sql"),
    ),
    (
        "0030_create_imap_uid_validity_table.sql",
        include_str!("../../migrations/0030_create_imap_uid_validity_table.sql"),
    ),
    (
        "0031_create_imap_mailboxes_table.sql",
        include_str!("../../migrations/0031_create_imap_mailboxes_table.sql"),
    ),
    (
        "0032_create_imap_mailbox_messages_table.sql",
        include_str!("../../migrations/0032_create_imap_mailbox_messages_table.sql"),
    ),
    (
        "0033_create_imap_mailbox_exclusions_table.sql",
        include_str!("../../migrations/0033_create_imap_mailbox_exclusions_table.sql"),
    ),
    (
        "0034_create_imap_message_flags_table.sql",
        include_str!("../../migrations/0034_create_imap_message_flags_table.sql"),
    ),
    (
        "0035_create_imap_unsubscribed_mailboxes_table.sql",
        include_str!("../../migrations/0035_create_imap_unsubscribed_mailboxes_table.sql"),
    ),
];

/// The version 1 endpoints script as written by pre-release builds with per-endpoint listeners.
///
/// It replaces the canonical baseline script only to recognize such databases; it is never applied.
pub(crate) const ENDPOINT_LISTENER_VARIANT: MigrationScript = (
    "0002_create_endpoints_table.sql",
    include_str!("../../migrations/variants/0002_create_endpoints_table_with_listeners.sql"),
);

/// Copies the primary per-endpoint listener into the global server for the listener variant.
pub(super) const COPY_ENDPOINT_LISTENER_ADDRESS: &str =
    include_str!("../../migrations/variants/0017_copy_endpoint_listener_address.sql");

/// A recognized physical layout of an existing database's schema version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SchemaLayout {
    /// Exactly the checked-in scripts of that version.
    Canonical,
    /// Version 1 created with per-endpoint SMTP listener columns on `endpoints`.
    EndpointListeners,
}

/// Every migration batch in version order; a batch's number is the schema version it produces.
pub(crate) const MIGRATION_BATCHES: [(i64, &[MigrationScript]); 5] = [
    (1, &BASELINE_MIGRATIONS),
    (2, &SMTP_ACCESS_MIGRATIONS),
    (3, &SINGLE_MAIL_POOL_MIGRATIONS),
    (4, &SMTP_ACCESS_AUTHENTICATION_RULES_MIGRATIONS),
    (5, &IMAP_MIGRATIONS),
];

/// Return the batches that produce the schema of `version`, in application order.
pub(crate) fn batches_through(
    version: i64,
) -> impl Iterator<Item = &'static (i64, &'static [MigrationScript])> {
    MIGRATION_BATCHES
        .iter()
        .filter(move |(batch, _)| *batch <= version)
}
