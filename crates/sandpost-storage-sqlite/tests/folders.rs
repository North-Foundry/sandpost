//! Ownership and initialization tests for IMAP and Web folder projections.
#[allow(dead_code)]
mod common;

use sandpost_core::{GlobalRole, ImapAccountMode, ImapFolderKind, MailAccess};
use sandpost_storage::{
    ImapLocalFolderSettings, ImapStorage, NewImapAccount, NewImapLocalFolder, NewUser, UserStorage,
};
use sandpost_storage_sqlite::SqliteStorage;

/// Return a unique file path for an isolated SQLite storage test.
fn database_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("sandpost-folders-{}.sqlite", uuid::Uuid::new_v4()))
}

/// Create an owner fixture with one account in the requested organization mode.
async fn account(
    storage: &SqliteStorage,
    owner: sandpost_core::UserIdentifier,
    mode: ImapAccountMode,
) -> sandpost_core::ImapAccount {
    storage
        .create_imap_account(&NewImapAccount {
            owner_identifier: owner,
            name: format!("{mode:?}"),
            username: format!("folders-{}", uuid::Uuid::new_v4()),
            password_hash: "hash".into(),
            mirror_views: false,
            mode,
            initial_authorization: sandpost_query::Expression::True,
            created_at: 1,
        })
        .await
        .unwrap()
}

/// Web folders snapshot privately for Independent accounts and project for every Mirror account.
#[tokio::test]
async fn folder_ownership_follows_account_mode() {
    let path = database_path();
    let storage = SqliteStorage::open(&path).await.unwrap();
    let owner = storage
        .create_first_owner(&NewUser {
            name: "Owner".into(),
            email: format!("folders-{}@example.test", uuid::Uuid::new_v4()),
            password_hash: "hash".into(),
            global_role: GlobalRole::Owner,
            mail_access: MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    let primary_folder = storage
        .create_web_folder(owner.identifier, "Primary", None, &[], 1)
        .await
        .unwrap();
    let mirror = account(&storage, owner.identifier, ImapAccountMode::Mirror).await;
    let independent = account(&storage, owner.identifier, ImapAccountMode::Independent).await;

    let mirror_folders = storage
        .list_imap_local_folders(mirror.identifier)
        .await
        .unwrap();
    let independent_folders = storage
        .list_imap_local_folders(independent.identifier)
        .await
        .unwrap();
    assert!(
        mirror_folders
            .iter()
            .any(|folder| folder.kind == ImapFolderKind::Manual && folder.name == "Primary")
    );
    assert!(
        independent_folders
            .iter()
            .any(|folder| folder.kind == ImapFolderKind::Private && folder.name == "Primary")
    );

    let independent_smart = storage
        .create_imap_local_folder(&NewImapLocalFolder {
            expected_mode_generation: None,
            account_identifier: independent.identifier,
            name: "Independent smart".into(),
            filter: "true".into(),
            include_in_inbox: false,
            created_at: 2,
        })
        .await
        .unwrap();
    assert_eq!(independent_smart.kind, ImapFolderKind::Smart);
    assert_eq!(independent_smart.definition_identifier, None);
    storage
        .update_imap_local_folder(
            independent_smart.identifier,
            &ImapLocalFolderSettings {
                expected_mode_generation: None,
                name: "Renamed local".into(),
                filter: "false".into(),
                include_in_inbox: true,
            },
        )
        .await
        .unwrap();
    assert!(
        storage
            .list_imap_local_folders(mirror.identifier)
            .await
            .unwrap()
            .iter()
            .all(|folder| folder.name != "Renamed local")
    );

    assert!(
        storage
            .get_web_folder(primary_folder.identifier)
            .await
            .unwrap()
            .is_some()
    );
    drop(storage);
    let _ = std::fs::remove_file(path);
}

/// Creating a Web folder projects it to existing Mirror accounts regardless of requested exposure.
#[tokio::test]
async fn new_web_folder_projects_to_mirror_accounts() {
    let path = database_path();
    let storage = SqliteStorage::open(&path).await.unwrap();
    let owner = storage
        .create_first_owner(&NewUser {
            name: "Owner".into(),
            email: format!("mirror-folders-{}@example.test", uuid::Uuid::new_v4()),
            password_hash: "hash".into(),
            global_role: GlobalRole::Owner,
            mail_access: MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    let mirror = account(&storage, owner.identifier, ImapAccountMode::Mirror).await;
    let independent = account(&storage, owner.identifier, ImapAccountMode::Independent).await;
    storage
        .create_web_folder(owner.identifier, "Automatic", None, &[], 1)
        .await
        .unwrap();

    assert!(
        storage
            .list_imap_local_folders(mirror.identifier)
            .await
            .unwrap()
            .iter()
            .any(|folder| folder.name == "Automatic")
    );
    assert!(
        storage
            .list_imap_local_folders(independent.identifier)
            .await
            .unwrap()
            .iter()
            .all(|folder| folder.name != "Automatic")
    );
    drop(storage);
    let _ = std::fs::remove_file(path);
}
