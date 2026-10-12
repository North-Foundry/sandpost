//! IMAP account credentials and account configuration checks.
use sandpost_core::{ImapLinkedView, UserIdentifier, View, ViewIdentifier};
use sandpost_storage::{
    ImapAccountSettings, ImapLocalFolderSettings, NewImapAccount, NewImapLocalFolder, Storage,
    StorageError,
};

use super::support::create_imap_account_fixture;
use crate::ConformanceFailure;
use crate::support::{failure, unique_token, unwrap_storage, verify, verify_equal};

/// Verify account creation, unique usernames, credential reads, settings, password replacement,
/// deletion, and cascading removal with the owner.
pub async fn imap_account_credentials(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "imap_account_credentials";
    let account = create_imap_account_fixture(storage, CHECK).await?;
    verify(CHECK, account.enabled, "new accounts are enabled")?;
    let missing_owner = storage
        .create_imap_account(&NewImapAccount {
            owner_identifier: UserIdentifier::new(),
            name: "Unknown owner".to_owned(),
            username: format!("imap-{}-{}", std::process::id(), unique_token()),
            password_hash: "hash".to_owned(),
            mirror_views: false,
            created_at: 1,
        })
        .await;
    verify(
        CHECK,
        matches!(missing_owner, Err(StorageError::NotFound)),
        format!("an unknown owner must fail with NotFound, got {missing_owner:?}"),
    )?;
    let duplicate = storage
        .create_imap_account(&NewImapAccount {
            owner_identifier: account.owner_identifier,
            name: "Duplicate".to_owned(),
            username: account.username.clone(),
            password_hash: "hash".to_owned(),
            mirror_views: false,
            created_at: 1,
        })
        .await;
    verify(
        CHECK,
        matches!(duplicate, Err(StorageError::DuplicateImapUsername)),
        format!("a taken username must fail with DuplicateImapUsername, got {duplicate:?}"),
    )?;
    let credential = unwrap_storage(
        CHECK,
        "get_imap_account_credential_by_username",
        storage
            .get_imap_account_credential_by_username(&account.username)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "credential by username is missing"))?;
    verify_equal(
        CHECK,
        "stored hash",
        credential.password_hash.as_str(),
        "hash-one",
    )?;
    verify_equal(CHECK, "credential account", &credential.account, &account)?;
    let updated = unwrap_storage(
        CHECK,
        "update_imap_account",
        storage
            .update_imap_account(
                account.identifier,
                &ImapAccountSettings {
                    name: "Renamed".to_owned(),
                    enabled: false,
                    mirror_views: false,
                },
            )
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "update returned no account"))?;
    verify(
        CHECK,
        updated.name == "Renamed" && !updated.enabled && !updated.mirror_views,
        format!("settings were not replaced: {updated:?}"),
    )?;
    unwrap_storage(
        CHECK,
        "replace_imap_account_password",
        storage
            .replace_imap_account_password(account.identifier, "hash-two")
            .await,
    )?;
    let credential = unwrap_storage(
        CHECK,
        "get_imap_account_credential",
        storage
            .get_imap_account_credential(account.identifier)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "credential by identifier is missing"))?;
    verify_equal(
        CHECK,
        "replaced hash",
        credential.password_hash.as_str(),
        "hash-two",
    )?;
    unwrap_storage(
        CHECK,
        "record_imap_account_use",
        storage
            .record_imap_account_use(account.identifier, 1_700_000_500)
            .await,
    )?;
    let listed = unwrap_storage(
        CHECK,
        "list_imap_accounts",
        storage.list_imap_accounts(account.owner_identifier).await,
    )?;
    verify_equal(CHECK, "listed accounts", listed.len(), 1)?;
    verify_equal(
        CHECK,
        "last use",
        listed[0].last_used_at,
        Some(1_700_000_500),
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "delete_user",
            storage.delete_user(account.owner_identifier).await,
        )?,
        "deleting the owner succeeds",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "get_imap_account after owner deletion",
            storage.get_imap_account(account.identifier).await,
        )?
        .is_none(),
        "deleting the owner deletes its IMAP accounts",
    )
}

/// Verify linked View selection and local folder CRUD, including cascades and duplicates.
pub async fn imap_account_configuration(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "imap_account_configuration";
    let account = create_imap_account_fixture(storage, CHECK).await?;
    let view = View {
        identifier: ViewIdentifier::new(),
        owner_identifier: Some(account.owner_identifier),
        name: "Laravel".to_owned(),
        filter: "subject contains \"laravel\"".to_owned(),
    };
    unwrap_storage(CHECK, "save_view", storage.save_view(&view).await)?;
    let missing = storage
        .set_imap_linked_views(
            account.identifier,
            &[ImapLinkedView {
                view_identifier: ViewIdentifier::new(),
                alias: None,
            }],
        )
        .await;
    verify(
        CHECK,
        matches!(missing, Err(StorageError::NotFound)),
        format!("an unknown View must fail with NotFound, got {missing:?}"),
    )?;
    let selection = vec![ImapLinkedView {
        view_identifier: view.identifier,
        alias: Some("Framework".to_owned()),
    }];
    unwrap_storage(
        CHECK,
        "set_imap_linked_views",
        storage
            .set_imap_linked_views(account.identifier, &selection)
            .await,
    )?;
    verify_equal(
        CHECK,
        "linked views",
        unwrap_storage(
            CHECK,
            "list_imap_linked_views",
            storage.list_imap_linked_views(account.identifier).await,
        )?,
        selection,
    )?;
    unwrap_storage(
        CHECK,
        "delete_view",
        storage.delete_view(view.identifier).await,
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "list_imap_linked_views after delete",
            storage.list_imap_linked_views(account.identifier).await,
        )?
        .is_empty(),
        "deleting a View removes it from linked selections",
    )?;
    let folder = unwrap_storage(
        CHECK,
        "create_imap_local_folder",
        storage
            .create_imap_local_folder(&NewImapLocalFolder {
                account_identifier: account.identifier,
                name: "Failed Payments".to_owned(),
                filter: "subject contains \"failed\"".to_owned(),
                include_in_inbox: true,
                created_at: 1,
            })
            .await,
    )?;
    let duplicate = storage
        .create_imap_local_folder(&NewImapLocalFolder {
            account_identifier: account.identifier,
            name: "Failed Payments".to_owned(),
            filter: String::new(),
            include_in_inbox: false,
            created_at: 1,
        })
        .await;
    verify(
        CHECK,
        matches!(duplicate, Err(StorageError::DuplicateImapFolderName)),
        format!("a duplicate folder name must fail, got {duplicate:?}"),
    )?;
    let updated = unwrap_storage(
        CHECK,
        "update_imap_local_folder",
        storage
            .update_imap_local_folder(
                folder.identifier,
                &ImapLocalFolderSettings {
                    name: "Password Resets".to_owned(),
                    filter: "subject contains \"reset\"".to_owned(),
                    include_in_inbox: false,
                },
            )
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "folder update returned nothing"))?;
    verify(
        CHECK,
        updated.name == "Password Resets" && !updated.include_in_inbox,
        format!("folder settings were not replaced: {updated:?}"),
    )?;
    let sibling = unwrap_storage(
        CHECK,
        "create sibling account",
        storage
            .create_imap_account(&NewImapAccount {
                owner_identifier: account.owner_identifier,
                name: "Sibling".to_owned(),
                username: format!("imap-{}-{}", std::process::id(), unique_token()),
                password_hash: "hash".to_owned(),
                mirror_views: false,
                created_at: 2,
            })
            .await,
    )?;
    let sibling_folder = unwrap_storage(
        CHECK,
        "shared folder projection",
        storage.list_imap_local_folders(sibling.identifier).await,
    )?
    .into_iter()
    .next()
    .ok_or_else(|| failure(CHECK, "new account did not discover owner folders"))?;
    verify(
        CHECK,
        sibling_folder.name == updated.name && sibling_folder.filter == updated.filter,
        format!("new account projection differs from owner definition: {sibling_folder:?}"),
    )?;
    let renamed = unwrap_storage(
        CHECK,
        "update shared folder through sibling account",
        storage
            .update_imap_local_folder(
                sibling_folder.identifier,
                &ImapLocalFolderSettings {
                    name: "Owner-wide Folder".to_owned(),
                    filter: "subject contains \"owner-wide\"".to_owned(),
                    include_in_inbox: true,
                },
            )
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "shared folder update returned nothing"))?;
    let first_projection = unwrap_storage(
        CHECK,
        "read canonical folder from first account",
        storage.list_imap_local_folders(account.identifier).await,
    )?
    .into_iter()
    .next()
    .ok_or_else(|| failure(CHECK, "first account lost its folder projection"))?;
    verify(
        CHECK,
        first_projection.name == renamed.name && first_projection.filter == renamed.filter,
        "updates through one account synchronize every projection",
    )?;
    unwrap_storage(
        CHECK,
        "delete sibling account",
        storage.delete_imap_account(sibling.identifier).await,
    )?;
    let replacement = unwrap_storage(
        CHECK,
        "create replacement account",
        storage
            .create_imap_account(&NewImapAccount {
                owner_identifier: account.owner_identifier,
                name: "Replacement".to_owned(),
                username: format!("imap-{}-{}", std::process::id(), unique_token()),
                password_hash: "hash".to_owned(),
                mirror_views: false,
                created_at: 3,
            })
            .await,
    )?;
    let replacement_folder = unwrap_storage(
        CHECK,
        "discover retained owner folder",
        storage
            .list_imap_local_folders(replacement.identifier)
            .await,
    )?
    .into_iter()
    .next()
    .ok_or_else(|| failure(CHECK, "credential deletion removed the owner definition"))?;
    verify_equal(
        CHECK,
        "listed folders",
        unwrap_storage(
            CHECK,
            "list_imap_local_folders",
            storage.list_imap_local_folders(account.identifier).await,
        )?,
        vec![first_projection],
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "delete_imap_local_folder",
            storage
                .delete_imap_local_folder(replacement_folder.identifier)
                .await,
        )?,
        "deleting a folder reports success",
    )?;
    unwrap_storage(
        CHECK,
        "set_imap_subscription",
        storage
            .set_imap_subscription(account.identifier, "Laravel", false)
            .await,
    )?;
    verify_equal(
        CHECK,
        "unsubscribed names",
        unwrap_storage(
            CHECK,
            "list_imap_unsubscribed",
            storage.list_imap_unsubscribed(account.identifier).await,
        )?,
        vec!["Laravel".to_owned()],
    )?;
    unwrap_storage(
        CHECK,
        "resubscribe",
        storage
            .set_imap_subscription(account.identifier, "Laravel", true)
            .await,
    )?;
    unwrap_storage(
        CHECK,
        "delete_imap_account",
        storage.delete_imap_account(account.identifier).await,
    )?;
    Ok(())
}
