//! Real-command IMAP tests for private mailboxes, Web placements, and exposed folders.
use super::*;
use sandpost_application::{ImapFolderRequest, QueryOptions, WebFolderRequest};
use sandpost_core::{ImapAccountIdentifier, ImapAccountMode};

/// Create an owner and the requested number of accounts in one organization mode.
async fn accounts(
    running: &RunningSandpost,
    count: usize,
    mode: ImapAccountMode,
) -> (SessionUser, Vec<IssuedImapCredentials>) {
    let owner_session = owner(running).await;
    let caller = running.application.authorized(&owner_session);
    let mut issued = Vec::new();
    for index in 0..count {
        issued.push(
            caller
                .create_imap_account(NewImapAccountRequest {
                    name: format!("Account {index}"),
                    username: None,
                    mirror_views: true,
                    mode: Some(mode),
                    linked_views: Vec::new(),
                })
                .await
                .unwrap(),
        );
    }
    (owner_session, issued)
}

/// Private folder CRUD preserves mailbox identity and message UIDs across rename and restart.
#[tokio::test]
async fn private_mailbox_crud_uidvalidity_and_restart() {
    let (configuration, _directory) = configuration();
    let running = SandpostRuntime::start(configuration.clone()).await.unwrap();
    let (_, issued) = accounts(&running, 1, ImapAccountMode::Independent).await;
    let credential = &issued[0];
    let smtp = running.mail_listen_address.unwrap();
    send_mail(smtp, "dev@example.test", "Private folder persistence").await;
    wait_for_mail(&running, 1).await;

    let mut client = Client::login(imap_address(&running), credential).await;
    let created = client.command("c", "CREATE Private").await;
    assert!(created.last().unwrap().starts_with("c OK"), "{created:?}");
    client.command("s", "SELECT INBOX").await;
    let copied = client.command("p", "UID COPY 1 Private").await;
    assert!(
        copied.last().unwrap().contains("[COPYUID ") && copied.last().unwrap().contains(" 1 1]"),
        "COPYUID maps source UID 1 to destination UID 1: {copied:?}"
    );
    let selected = client.command("d", "SELECT Private").await;
    let validity = uid_validity(&selected);
    assert!(
        selected.contains(&"* 1 EXISTS\r\n".to_owned()),
        "{selected:?}"
    );
    assert!(selected.contains(&"* OK [UIDNEXT 2] Predicted next UID\r\n".to_owned()));

    client.command("u", "UNSELECT").await;
    let renamed = client.command("r", "RENAME Private Renamed").await;
    assert!(renamed.last().unwrap().starts_with("r OK"), "{renamed:?}");
    let selected = client.command("n", "SELECT Renamed").await;
    assert_eq!(uid_validity(&selected), validity);
    assert!(
        client
            .command("f", "UID FETCH 1 (UID)")
            .await
            .iter()
            .any(|line| line.contains("UID 1")),
        "UID remains stable after rename"
    );
    drop(client);
    running.shutdown().await.unwrap();

    let restarted = SandpostRuntime::start(configuration).await.unwrap();
    let mut client = Client::login(imap_address(&restarted), credential).await;
    let selected = client.command("s", "SELECT Renamed").await;
    assert_eq!(uid_validity(&selected), validity);
    assert!(
        selected.contains(&"* 1 EXISTS\r\n".to_owned()),
        "{selected:?}"
    );
    assert!(
        client
            .command("f", "UID FETCH 1 (UID)")
            .await
            .iter()
            .any(|line| line.contains("UID 1"))
    );
    restarted.shutdown().await.unwrap();
}

/// Mirror normal folders share Web structure changes while smart folders refuse edits and insertion.
#[tokio::test]
async fn mirror_manual_structure_and_smart_insertion_rights() {
    let (configuration, _directory) = configuration();
    let running = SandpostRuntime::start(configuration).await.unwrap();
    let (owner_session, issued) = accounts(&running, 1, ImapAccountMode::Mirror).await;
    let account_id: ImapAccountIdentifier = issued[0].account.account.identifier;
    let caller = running.application.authorized(&owner_session);
    let smart = caller
        .create_imap_folder(
            account_id,
            ImapFolderRequest {
                name: "Matching Smart".into(),
                filter: "subject contains \"Matching target\"".into(),
                include_in_inbox: false,
            },
        )
        .await
        .unwrap();
    let manual = caller
        .create_web_folder(WebFolderRequest {
            name: "Web Manual".into(),
            exposed_accounts: vec![account_id],
        })
        .await
        .unwrap();
    send_mail(
        running.mail_listen_address.unwrap(),
        "dev@example.test",
        "Matching target message",
    )
    .await;
    wait_for_mail(&running, 1).await;

    let mut client = Client::login(imap_address(&running), &issued[0]).await;
    let renamed = client
        .command("r1", "RENAME \"Web Manual\" \"Web Renamed\"")
        .await;
    assert!(renamed.last().unwrap().starts_with("r1 OK"), "{renamed:?}");
    assert!(
        caller
            .list_web_folders()
            .await
            .unwrap()
            .iter()
            .any(|folder| folder.identifier == manual.identifier && folder.name == "Web Renamed")
    );
    let deleted = client.command("d1", "DELETE \"Web Renamed\"").await;
    assert!(deleted.last().unwrap().starts_with("d1 OK"), "{deleted:?}");
    assert!(
        caller
            .list_web_folders()
            .await
            .unwrap()
            .iter()
            .all(|folder| folder.identifier != manual.identifier)
    );
    for (tag, command) in [
        ("d2", "DELETE \"Matching Smart\""),
        ("r2", "RENAME \"Matching Smart\" Other"),
    ] {
        let response = client.command(tag, command).await;
        assert!(
            response
                .last()
                .unwrap()
                .starts_with(&format!("{tag} NO [CANNOT]")),
            "{command}: {response:?}"
        );
    }
    for (tag, command) in [("c2", "CREATE \"Matching Smart\"")] {
        let response = client.command(tag, command).await;
        assert!(
            response.last().unwrap().starts_with(&format!("{tag} NO")),
            "{command}: {response:?}"
        );
    }
    client.command("s", "SELECT INBOX").await;
    for (tag, command) in [
        ("p", "COPY 1 \"Matching Smart\""),
        ("m", "MOVE 1 \"Matching Smart\""),
    ] {
        let response = client.command(tag, command).await;
        assert!(
            response
                .last()
                .unwrap()
                .starts_with(&format!("{tag} NO [CANNOT]")),
            "matching smart insertion: {response:?}"
        );
    }
    assert_eq!(manual.name, "Web Manual");
    assert_eq!(smart.name, "Matching Smart");
    running.shutdown().await.unwrap();
}

/// Mirror follows current primary folders while Independent keeps its creation snapshot.
#[tokio::test]
async fn mirror_follows_web_placement_and_independent_keeps_its_snapshot() {
    let (configuration, _directory) = configuration();
    let running = SandpostRuntime::start(configuration).await.unwrap();
    let (owner_session, issued) = accounts(&running, 2, ImapAccountMode::Independent).await;
    let account_a = issued[0].account.account.identifier;
    let caller = running.application.authorized(&owner_session);
    caller
        .update_imap_account(
            account_a,
            sandpost_application::ImapAccountUpdate {
                mode: Some(ImapAccountMode::Mirror),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let folder = caller
        .create_web_folder(WebFolderRequest {
            name: "Primary Folder".into(),
            exposed_accounts: vec![],
        })
        .await
        .unwrap();
    send_mail(
        running.mail_listen_address.unwrap(),
        "dev@example.test",
        "Web placement inheritance",
    )
    .await;
    wait_for_mail(&running, 1).await;
    let message = caller
        .query_web_mailbox(QueryOptions::default(), None)
        .await
        .unwrap()
        .into_iter()
        .find(|message| message.subject == "Web placement inheritance")
        .unwrap();
    assert_eq!(
        caller
            .move_web_messages(vec![message.identifier], Some(folder.identifier))
            .await
            .unwrap(),
        1
    );

    let mut client_a = Client::login(imap_address(&running), &issued[0]).await;
    let listings = client_a.command("l", "LIST \"\" *").await;
    assert!(
        listings.iter().any(|line| line.contains("Primary Folder")),
        "{listings:?}"
    );
    client_a.command("s", "SELECT \"Primary Folder\"").await;
    assert_eq!(
        client_a.command("q", "SEARCH ALL").await[0],
        "* SEARCH 1\r\n"
    );

    let mut client_b = Client::login(imap_address(&running), &issued[1]).await;
    let listings = client_b.command("l", "LIST \"\" *").await;
    assert!(
        listings.iter().all(|line| !line.contains("Primary Folder")),
        "{listings:?}"
    );
    client_b.command("s", "SELECT INBOX").await;
    assert_eq!(
        client_b.command("q", "SEARCH ALL").await[0],
        "* SEARCH 1\r\n"
    );
    assert_eq!(
        caller
            .query_web_mailbox(QueryOptions::default(), Some(folder.identifier))
            .await
            .unwrap()
            .iter()
            .map(|message| message.identifier)
            .collect::<Vec<_>>(),
        [message.identifier]
    );
    running.shutdown().await.unwrap();
}

/// Private MOVE stays local through Web/filter changes; deleting its last normal folder trashes mail.
#[tokio::test]
async fn private_move_isolation_overrides_and_safe_folder_deletion() {
    let (configuration, _directory) = configuration();
    let running = SandpostRuntime::start(configuration).await.unwrap();
    let (owner_session, issued) = accounts(&running, 2, ImapAccountMode::Independent).await;
    let account_a = issued[0].account.account.identifier;
    let caller = running.application.authorized(&owner_session);
    let smart = caller
        .create_imap_folder(
            account_a,
            ImapFolderRequest {
                name: "Editable Smart".into(),
                filter: "true".into(),
                include_in_inbox: false,
            },
        )
        .await
        .unwrap();
    let web_folder = caller
        .create_web_folder(WebFolderRequest {
            name: "Web Placement".into(),
            exposed_accounts: vec![account_a],
        })
        .await
        .unwrap();
    send_mail(
        running.mail_listen_address.unwrap(),
        "dev@example.test",
        "Private override target",
    )
    .await;
    wait_for_mail(&running, 1).await;
    let message = caller
        .query_web_mailbox(QueryOptions::default(), None)
        .await
        .unwrap()
        .into_iter()
        .find(|message| message.subject == "Private override target")
        .unwrap();

    let mut client_a = Client::login(imap_address(&running), &issued[0]).await;
    assert!(
        client_a
            .command("c", "CREATE Private")
            .await
            .last()
            .unwrap()
            .starts_with("c OK")
    );
    client_a.command("s", "SELECT INBOX").await;
    let moved = client_a.command("m", "UID MOVE 1 Private").await;
    assert!(
        moved.iter().any(|line| line.contains("[COPYUID ")),
        "{moved:?}"
    );
    client_a.command("p", "EXAMINE Private").await;
    assert!(
        client_a
            .command("st", "STORE 1 +FLAGS (\\Seen)")
            .await
            .last()
            .unwrap()
            .starts_with("st NO")
    );
    assert!(
        client_a
            .command("mv", "UID MOVE 1 Trash")
            .await
            .last()
            .unwrap()
            .starts_with("mv NO")
    );

    let mut client_b = Client::login(imap_address(&running), &issued[1]).await;
    client_b.command("s", "SELECT INBOX").await;
    assert_eq!(
        client_b.command("q", "SEARCH ALL").await[0],
        "* SEARCH 1\r\n"
    );
    assert_eq!(
        caller
            .query_web_mailbox(QueryOptions::default(), None)
            .await
            .unwrap()
            .iter()
            .map(|item| item.identifier)
            .collect::<Vec<_>>(),
        [message.identifier]
    );

    caller
        .update_imap_folder(
            account_a,
            smart.identifier,
            ImapFolderRequest {
                name: "Editable Smart".into(),
                filter: "subject contains \"no longer matches\"".into(),
                include_in_inbox: false,
            },
        )
        .await
        .unwrap();
    caller
        .move_web_messages(vec![message.identifier], Some(web_folder.identifier))
        .await
        .unwrap();
    client_b.command("b2", "SELECT INBOX").await;
    assert_eq!(
        client_b.command("q2", "SEARCH ALL").await[0],
        "* SEARCH 1\r\n"
    );
    client_a.command("n", "SELECT Private").await;
    assert!(
        client_a
            .command("f", "UID FETCH 1 (UID)")
            .await
            .iter()
            .any(|line| line.contains("UID 1"))
    );

    client_a.command("u", "UNSELECT").await;
    let deleted_private = client_a.command("d", "DELETE Private").await;
    assert!(
        deleted_private.last().unwrap().starts_with("d OK"),
        "{deleted_private:?}"
    );
    client_a.command("i", "SELECT INBOX").await;
    assert_eq!(
        client_a.command("q", "SEARCH ALL").await[0],
        "* SEARCH\r\n",
        "deleting the final normal occurrence leaves Independent Inbox empty"
    );
    client_a.command("w", "SELECT Trash").await;
    assert_eq!(
        client_a.command("qw", "SEARCH ALL").await[0],
        "* SEARCH 1\r\n"
    );
    caller
        .delete_web_folder(web_folder.identifier)
        .await
        .unwrap();
    client_a.command("i2", "SELECT Trash").await;
    assert_eq!(
        caller
            .query_web_trash(QueryOptions::default())
            .await
            .unwrap()
            .iter()
            .map(|item| item.identifier)
            .collect::<Vec<_>>(),
        [message.identifier]
    );
    assert_eq!(
        client_a.command("q2", "SEARCH ALL").await[0],
        "* SEARCH 1\r\n"
    );
    assert!(
        client_a
            .command("f", "UID FETCH 1:* (UID)")
            .await
            .iter()
            .any(|line| line.contains("FETCH (UID")),
        "deleting folders preserves the underlying mail"
    );
    running.shutdown().await.unwrap();
}
