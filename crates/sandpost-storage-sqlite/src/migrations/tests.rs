//! Validate every checked-in inverse against its preceding physical schema.
use super::*;
use sqlx::{ConnectOptions, sqlite::SqliteConnectOptions};

/// Each inverse restores its exact predecessor, including modified indexes and triggers.
#[tokio::test]
async fn every_inverse_restores_the_preceding_schema() {
    let mut connection = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true)
        .connect()
        .await
        .unwrap();
    migrate(&mut connection).await.unwrap();
    for index in (0..MIGRATIONS.len()).rev() {
        execute_script(&mut connection, MIGRATIONS[index].0, ROLLBACKS[index])
            .await
            .unwrap();
        assert!(
            schema::matches_schema(&mut connection, index, false)
                .await
                .unwrap(),
            "inverse of {} must restore its predecessor",
            MIGRATIONS[index].0
        );
        validate_foreign_keys(&mut connection).await.unwrap();
    }
    migrate(&mut connection).await.unwrap();
    assert_eq!(
        status(&mut connection).await.unwrap().len(),
        MIGRATIONS.len()
    );
}

/// Populated Mirror and Independent state can be removed by the consolidated rollback sequence.
#[tokio::test]
async fn populated_mode_separated_state_rolls_back_cleanly() {
    let mut connection = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true)
        .connect()
        .await
        .unwrap();
    migrate(&mut connection).await.unwrap();
    sqlx::raw_sql(
        "INSERT INTO users(identifier, name, email, password_hash, global_role, mail_access, created_at, updated_at)
         VALUES ('owner', 'Owner', 'owner@example.test', 'hash', 'owner', 'all', 1, 1);
         INSERT INTO imap_accounts(identifier, owner_identifier, name, username, password_hash, created_at, updated_at, mode)
         VALUES ('mirror-account', 'owner', 'Mirror account', 'mirror-account', 'hash', 1, 1, 'mirror'),
                ('independent-account', 'owner', 'Independent account', 'independent-account', 'hash', 1, 1, 'independent');
         INSERT INTO imap_user_folders(identifier, owner_identifier, name, filter, include_in_inbox, created_at, updated_at)
         VALUES ('mirror-definition', 'owner', 'Mirror folder', '', 0, 1, 1);
         INSERT INTO imap_local_folders(identifier, account_identifier, name, filter, include_in_inbox, created_at, updated_at, kind, definition_identifier, organization_mode)
         VALUES ('mirror-folder', 'mirror-account', '__sandpost_owner_folder_mirror-definition', '', 0, 1, 1, 'smart', 'mirror-definition', 'mirror'),
                ('independent-folder', 'independent-account', 'Independent folder', '', 0, 1, 1, 'private', NULL, 'independent');
         INSERT INTO imap_user_folder_accounts(account_identifier, folder_identifier, local_folder_identifier)
         VALUES ('mirror-account', 'mirror-definition', 'mirror-folder');
         INSERT INTO imap_mailboxes(identifier, account_identifier, source, folder_identifier, uid_validity, organization_mode)
         VALUES ('mirror-mailbox', 'mirror-account', 'folder', 'mirror-folder', 1, 'mirror'),
                ('independent-mailbox', 'independent-account', 'folder', 'independent-folder', 2, 'independent');",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    let reverted = rollback(&mut connection).await.unwrap();
    assert_eq!(reverted.len(), MIGRATIONS.len());
    assert!(status(&mut connection).await.unwrap().is_empty());
    assert!(schema::schema_is_empty(&mut connection).await.unwrap());
    validate_foreign_keys(&mut connection).await.unwrap();
}
