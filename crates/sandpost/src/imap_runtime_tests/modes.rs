//! Wire coverage of shared primary organization, dormant Independent state, and hierarchy UIDs.
use super::*;
use sandpost_application::{ImapAccountUpdate, QueryOptions};
use sandpost_core::ImapAccountMode;

/// Issue credentials in the requested organization mode for the runtime's owner.
async fn create_account(
    running: &RunningSandpost,
    owner: &SessionUser,
    name: &str,
    mode: ImapAccountMode,
) -> IssuedImapCredentials {
    running
        .application
        .authorized(owner)
        .create_imap_account(NewImapAccountRequest {
            name: name.into(),
            username: None,
            mode: Some(mode),
            mirror_views: true,
            linked_views: Vec::new(),
        })
        .await
        .unwrap()
}

/// Live mode changes invalidate sessions, preserve dormant state, and restore its UID namespace.
#[tokio::test]
async fn mode_switches_revoke_wire_sessions_and_restore_dormant_uids() {
    let (configuration, _directory) = configuration();
    let running = SandpostRuntime::start(configuration.clone()).await.unwrap();
    let owner = owner(&running).await;
    let caller = running.application.authorized(&owner);
    let issued = create_account(
        &running,
        &owner,
        "Switch modes",
        ImapAccountMode::Independent,
    )
    .await;
    let identifier = issued.account.account.identifier;
    send_mail(
        running.mail_listen_address,
        "dev@example.test",
        "Before switching",
    )
    .await;
    wait_for_mail(&running, 1).await;
    let mut independent = Client::login(imap_address(&running), &issued).await;
    assert!(
        independent
            .command("c", "CREATE Private")
            .await
            .last()
            .unwrap()
            .starts_with("c OK")
    );
    let inbox = independent.command("s", "SELECT INBOX").await;
    let independent_inbox_validity = uid_validity(&inbox);
    let moved = independent.command("m", "UID MOVE 1 Private").await;
    assert!(
        moved
            .iter()
            .any(|line| line.contains("[COPYUID ") && line.contains(" 1 1]")),
        "{moved:?}"
    );
    let private = independent.command("p", "SELECT Private").await;
    let private_validity = uid_validity(&private);
    independent
        .command("f", "UID STORE 1 +FLAGS (\\Seen)")
        .await;
    independent
        .0
        .get_mut()
        .write_all(b"i IDLE\r\n")
        .await
        .unwrap();
    assert_eq!(independent.response().await, "+ idling\r\n");
    caller
        .update_imap_account(
            identifier,
            ImapAccountUpdate {
                mode: Some(ImapAccountMode::Mirror),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(independent.response().await.starts_with("* BYE"));

    let mut mirror = Client::login(imap_address(&running), &issued).await;
    let listings = mirror.command("l", "LIST \"\" *").await;
    assert!(
        listings.iter().all(|line| !line.contains("Private")),
        "{listings:?}"
    );
    let primary = mirror.command("s", "SELECT INBOX").await;
    assert_ne!(uid_validity(&primary), independent_inbox_validity);
    assert_eq!(
        mirror.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 1\r\n"
    );
    assert!(
        mirror
            .command("f", "UID FETCH 1 (FLAGS)")
            .await
            .iter()
            .all(|line| !line.contains("\\Seen"))
    );
    mirror.0.get_mut().write_all(b"i IDLE\r\n").await.unwrap();
    assert_eq!(mirror.response().await, "+ idling\r\n");
    send_mail(
        running.mail_listen_address,
        "dev@example.test",
        "While Independent is dormant",
    )
    .await;
    assert_eq!(mirror.response().await, "* 2 EXISTS\r\n");
    assert_eq!(mirror.response().await, "* 2 RECENT\r\n");
    mirror.0.get_mut().write_all(b"DONE\r\n").await.unwrap();
    assert_eq!(mirror.response().await, "i OK IDLE terminated\r\n");
    caller
        .update_imap_account(
            identifier,
            ImapAccountUpdate {
                mode: Some(ImapAccountMode::Independent),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    mirror.0.get_mut().write_all(b"n NOOP\r\n").await.unwrap();
    assert!(mirror.response().await.starts_with("* BYE"));

    let mut restored = Client::login(imap_address(&running), &issued).await;
    let private = restored.command("p", "SELECT Private").await;
    assert_eq!(uid_validity(&private), private_validity);
    assert!(private.contains(&"* OK [UIDNEXT 2] Predicted next UID\r\n".to_owned()));
    let flags = restored.command("f", "UID FETCH 1 (UID FLAGS)").await;
    assert!(
        flags
            .iter()
            .any(|line| line.contains("UID 1") && line.contains("\\Seen")),
        "{flags:?}"
    );
    let inbox = restored.command("s", "SELECT INBOX").await;
    assert_eq!(uid_validity(&inbox), independent_inbox_validity);
    assert!(inbox.contains(&"* OK [UIDNEXT 3] Predicted next UID\r\n".to_owned()));
    assert_eq!(
        restored.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 2\r\n"
    );
    for mode in [ImapAccountMode::Mirror, ImapAccountMode::Independent] {
        caller
            .update_imap_account(
                identifier,
                ImapAccountUpdate {
                    mode: Some(mode),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
    restored.0.get_mut().write_all(b"n NOOP\r\n").await.unwrap();
    assert!(
        restored.response().await.starts_with("* BYE"),
        "switching away and back must invalidate the earlier generation"
    );
    drop(independent);
    drop(mirror);
    drop(restored);
    running.shutdown().await.unwrap();
    let restarted = SandpostRuntime::start(configuration).await.unwrap();
    let mut client = Client::login(imap_address(&restarted), &issued).await;
    assert_eq!(
        uid_validity(&client.command("s", "SELECT Private").await),
        private_validity
    );
    assert_eq!(
        client.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 1\r\n"
    );
    assert_eq!(
        uid_validity(&client.command("s", "SELECT INBOX").await),
        independent_inbox_validity
    );
    assert_eq!(
        client.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 2\r\n"
    );
    restarted.shutdown().await.unwrap();
}

/// Mirror credentials share primary folders, Seen, and Trash while Independent remains isolated.
#[tokio::test]
async fn mirror_wire_mutations_share_web_organization_and_preserve_independent_state() {
    let (configuration, _directory) = configuration();
    let running = SandpostRuntime::start(configuration).await.unwrap();
    let owner = owner(&running).await;
    let caller = running.application.authorized(&owner);
    let first = create_account(&running, &owner, "Mirror first", ImapAccountMode::Mirror).await;
    let second = create_account(&running, &owner, "Mirror second", ImapAccountMode::Mirror).await;
    let private = create_account(
        &running,
        &owner,
        "Independent",
        ImapAccountMode::Independent,
    )
    .await;
    send_mail(
        running.mail_listen_address,
        "dev@example.test",
        "Shared primary organization",
    )
    .await;
    wait_for_mail(&running, 1).await;
    let message = caller
        .query_web_mailbox(QueryOptions::default(), None)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let mut first = Client::login(imap_address(&running), &first).await;
    let mut second = Client::login(imap_address(&running), &second).await;
    let mut independent = Client::login(imap_address(&running), &private).await;
    assert!(
        first
            .command("c", "CREATE Shared")
            .await
            .last()
            .unwrap()
            .starts_with("c OK")
    );
    let folder = caller
        .list_web_folders()
        .await
        .unwrap()
        .into_iter()
        .find(|folder| folder.name == "Shared")
        .unwrap();
    assert!(
        second
            .command("l", "LIST \"\" *")
            .await
            .iter()
            .any(|line| line.ends_with("Shared\r\n"))
    );
    assert!(
        independent
            .command("l", "LIST \"\" *")
            .await
            .iter()
            .all(|line| !line.contains("Shared"))
    );
    first.command("s", "SELECT INBOX").await;
    let moved = first.command("m", "UID MOVE 1 Shared").await;
    assert!(
        moved.iter().any(|line| line.contains("[COPYUID ")),
        "{moved:?}"
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
    first.command("s", "SELECT Shared").await;
    let shared = second.command("s", "SELECT Shared").await;
    let shared_validity = uid_validity(&shared);
    let snapshot_credentials = create_account(
        &running,
        &owner,
        "Snapshot current primary",
        ImapAccountMode::Independent,
    )
    .await;
    let mut snapshot = Client::login(imap_address(&running), &snapshot_credentials).await;
    let snapshot_validity = uid_validity(&snapshot.command("s", "SELECT Shared").await);
    assert_eq!(
        snapshot.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 1\r\n"
    );
    assert_eq!(
        second.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 1\r\n"
    );
    first.command("s", "SELECT Shared").await;
    first.command("s", "SELECT Shared").await;
    assert_eq!(
        first.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 1\r\n"
    );
    assert!(
        first
            .command("f", "UID STORE 1 +FLAGS (\\Seen)")
            .await
            .last()
            .unwrap()
            .starts_with("f OK")
    );
    assert!(
        second
            .command("f", "UID FETCH 1 (FLAGS)")
            .await
            .iter()
            .any(|line| line.contains("\\Seen"))
    );
    independent.command("s", "SELECT INBOX").await;
    assert!(
        independent
            .command("f", "UID FETCH 1 (FLAGS)")
            .await
            .iter()
            .all(|line| !line.contains("\\Seen"))
    );
    independent.command("c", "CREATE Local").await;
    assert!(
        independent
            .command("c", "UID COPY 1 Local")
            .await
            .last()
            .unwrap()
            .starts_with("c OK")
    );
    independent.command("s", "SELECT Local").await;
    assert!(
        independent
            .command("f", "UID STORE 1 +FLAGS (\\Seen)")
            .await
            .last()
            .unwrap()
            .starts_with("f OK")
    );
    independent.command("s", "SELECT INBOX").await;
    assert!(
        independent
            .command("f", "UID FETCH 1 (FLAGS)")
            .await
            .iter()
            .any(|line| line.contains("\\Seen"))
    );
    assert!(
        first
            .command("r", "RENAME Shared Renamed")
            .await
            .last()
            .unwrap()
            .starts_with("r OK")
    );
    assert_eq!(
        uid_validity(&second.command("s", "SELECT Renamed").await),
        shared_validity
    );
    assert!(
        caller
            .list_web_folders()
            .await
            .unwrap()
            .iter()
            .any(|item| item.identifier == folder.identifier && item.name == "Renamed")
    );
    second.0.get_mut().write_all(b"i IDLE\r\n").await.unwrap();
    assert_eq!(second.response().await, "+ idling\r\n");
    caller
        .move_web_messages(vec![message.identifier], None)
        .await
        .unwrap();
    assert_eq!(second.response().await, "* 1 EXPUNGE\r\n");
    second.0.get_mut().write_all(b"DONE\r\n").await.unwrap();
    assert_eq!(second.response().await, "i OK IDLE terminated\r\n");
    first.command("s", "SELECT INBOX").await;
    assert_eq!(
        first.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 2\r\n"
    );
    let moved = first.command("m", "UID MOVE 2 Trash").await;
    assert!(
        moved.iter().any(|line| line.contains("[COPYUID ")),
        "{moved:?}"
    );
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
    second.command("s", "SELECT Trash").await;
    assert_eq!(
        second.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 1\r\n"
    );
    assert_eq!(
        independent.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 1\r\n"
    );
    assert_eq!(
        uid_validity(&snapshot.command("s", "SELECT Shared").await),
        snapshot_validity
    );
    assert_eq!(
        snapshot.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 1\r\n",
        "Independent preserves the folder snapshot through primary rename, move, and Trash"
    );
    assert!(
        snapshot
            .command("f", "UID FETCH 1 (BODY.PEEK[])")
            .await
            .iter()
            .any(|line| line.contains("Shared primary organization"))
    );
    running.shutdown().await.unwrap();
}

/// Coalesce an existing Trash occurrence during an Apple Mail style bulk COPY in both modes.
#[tokio::test]
async fn bulk_copy_to_trash_coalesces_existing_occurrence_in_both_modes() {
    for mode in [ImapAccountMode::Mirror, ImapAccountMode::Independent] {
        let (configuration, _directory) = configuration();
        let running = SandpostRuntime::start(configuration).await.unwrap();
        let owner = owner(&running).await;
        let issued = create_account(&running, &owner, "Trash bulk copy", mode).await;
        let smtp = running.mail_listen_address;
        for subject in ["Bulk delete one", "Bulk delete two", "Bulk delete three"] {
            send_mail(smtp, "dev@example.test", subject).await;
        }
        wait_for_mail(&running, 3).await;

        let mut client = Client::login(imap_address(&running), &issued).await;
        client.command("s", "SELECT INBOX").await;
        let initial_copy = client.command("c", "UID COPY 1 Trash").await;
        assert!(
            initial_copy.last().unwrap().starts_with("c OK"),
            "{initial_copy:?}"
        );
        assert!(
            initial_copy
                .iter()
                .any(|line| line.contains("[COPYUID ") && line.contains(" 1 1]")),
            "the initial copy establishes Trash UID 1: {initial_copy:?}"
        );

        for tag in ["b", "r"] {
            let copied = client.command(tag, "UID COPY 1:3 Trash").await;
            assert!(
                copied.last().unwrap().starts_with(&format!("{tag} OK")),
                "{copied:?}"
            );
            assert!(
                copied.iter().all(|line| !line.contains("[COPYUID ")),
                "a coalesced batch must not report a partial COPYUID mapping: {copied:?}"
            );
        }

        let marked = client
            .command("d", "UID STORE 1:3 +FLAGS (\\Deleted)")
            .await;
        assert!(marked.last().unwrap().starts_with("d OK"), "{marked:?}");
        let expunged = client.command("e", "UID EXPUNGE 1:3").await;
        assert!(expunged.last().unwrap().starts_with("e OK"), "{expunged:?}");
        drop(client);

        let mut client = Client::login(imap_address(&running), &issued).await;
        let inbox = client.command("i", "SELECT INBOX").await;
        assert!(inbox.contains(&"* OK [UIDNEXT 4] Predicted next UID\r\n".to_owned()));
        assert_eq!(
            client.command("q", "UID SEARCH ALL").await[0],
            "* SEARCH\r\n"
        );

        let trash = client.command("t", "SELECT Trash").await;
        assert!(trash.contains(&"* 3 EXISTS\r\n".to_owned()), "{trash:?}");
        assert!(trash.contains(&"* OK [UIDNEXT 4] Predicted next UID\r\n".to_owned()));
        let messages = client.command("f", "UID FETCH 1:3 (UID)").await;
        for uid in 1..=3 {
            assert!(
                messages
                    .iter()
                    .any(|line| line.contains(&format!("UID {uid}"))),
                "Trash must retain original UID {uid}: {messages:?}"
            );
        }

        let copied_back = client.command("x", "UID COPY 1 INBOX").await;
        assert!(
            copied_back.last().unwrap().starts_with("x OK"),
            "{copied_back:?}"
        );
        assert!(
            copied_back
                .iter()
                .any(|line| line.contains("[COPYUID ") && line.contains(" 1 4]")),
            "the additional Inbox occurrence must receive UID 4: {copied_back:?}"
        );
        client.command("j", "SELECT INBOX").await;
        let coalesced = client.command("k", "UID COPY 4 Trash").await;
        assert!(
            coalesced.last().unwrap().starts_with("k OK"),
            "{coalesced:?}"
        );
        assert!(
            coalesced.iter().all(|line| !line.contains("[COPYUID ")),
            "coalescing into canonical Trash must suppress COPYUID: {coalesced:?}"
        );
        let marked = client.command("l", "UID STORE 4 +FLAGS (\\Deleted)").await;
        assert!(marked.last().unwrap().starts_with("l OK"), "{marked:?}");
        let expunged = client.command("n", "UID EXPUNGE 4").await;
        assert!(expunged.last().unwrap().starts_with("n OK"), "{expunged:?}");
        drop(client);

        let mut client = Client::login(imap_address(&running), &issued).await;
        let inbox = client.command("o", "SELECT INBOX").await;
        assert!(inbox.contains(&"* OK [UIDNEXT 5] Predicted next UID\r\n".to_owned()));
        assert_eq!(
            client.command("p", "UID SEARCH ALL").await[0],
            "* SEARCH\r\n"
        );
        let trash = client.command("q", "SELECT Trash").await;
        assert!(trash.contains(&"* 3 EXISTS\r\n".to_owned()), "{trash:?}");
        assert!(trash.contains(&"* OK [UIDNEXT 4] Predicted next UID\r\n".to_owned()));
        let messages = client.command("r", "UID FETCH 1:3 (UID)").await;
        for uid in 1..=3 {
            assert!(
                messages
                    .iter()
                    .any(|line| line.contains(&format!("UID {uid}"))),
                "Trash must retain original UID {uid}: {messages:?}"
            );
        }
        drop(client);
        running.shutdown().await.unwrap();
    }
}

/// Coalesce an existing Trash occurrence during a MOVE batch in both organization modes.
#[tokio::test]
async fn bulk_move_to_trash_coalesces_existing_occurrence_in_both_modes() {
    for mode in [ImapAccountMode::Mirror, ImapAccountMode::Independent] {
        let (configuration, _directory) = configuration();
        let running = SandpostRuntime::start(configuration).await.unwrap();
        let owner = owner(&running).await;
        let issued = create_account(&running, &owner, "Trash bulk move", mode).await;
        let smtp = running.mail_listen_address;
        for subject in ["Bulk move one", "Bulk move two", "Bulk move three"] {
            send_mail(smtp, "dev@example.test", subject).await;
        }
        wait_for_mail(&running, 3).await;

        let mut client = Client::login(imap_address(&running), &issued).await;
        client.command("s", "SELECT INBOX").await;
        let initial_copy = client.command("c", "UID COPY 1 Trash").await;
        assert!(
            initial_copy.last().unwrap().starts_with("c OK"),
            "{initial_copy:?}"
        );
        assert!(
            initial_copy
                .iter()
                .any(|line| line.contains("[COPYUID ") && line.contains(" 1 1]")),
            "the initial copy establishes Trash UID 1: {initial_copy:?}"
        );

        let moved = client.command("m", "UID MOVE 1:3 Trash").await;
        assert!(moved.last().unwrap().starts_with("m OK"), "{moved:?}");
        assert!(
            moved.iter().all(|line| !line.contains("[COPYUID ")),
            "a coalesced MOVE batch must not report a partial COPYUID mapping: {moved:?}"
        );
        drop(client);

        let mut client = Client::login(imap_address(&running), &issued).await;
        let inbox = client.command("i", "SELECT INBOX").await;
        assert!(inbox.contains(&"* OK [UIDNEXT 4] Predicted next UID\r\n".to_owned()));
        assert_eq!(
            client.command("q", "UID SEARCH ALL").await[0],
            "* SEARCH\r\n"
        );

        let trash = client.command("t", "SELECT Trash").await;
        assert!(trash.contains(&"* 3 EXISTS\r\n".to_owned()), "{trash:?}");
        assert!(trash.contains(&"* OK [UIDNEXT 4] Predicted next UID\r\n".to_owned()));
        let messages = client.command("f", "UID FETCH 1:3 (UID)").await;
        for uid in 1..=3 {
            assert!(
                messages
                    .iter()
                    .any(|line| line.contains(&format!("UID {uid}"))),
                "Trash must retain original UID {uid}: {messages:?}"
            );
        }

        let copied_back = client.command("x", "UID COPY 1 INBOX").await;
        assert!(
            copied_back.last().unwrap().starts_with("x OK"),
            "{copied_back:?}"
        );
        assert!(
            copied_back
                .iter()
                .any(|line| line.contains("[COPYUID ") && line.contains(" 1 4]")),
            "the additional Inbox occurrence must receive UID 4: {copied_back:?}"
        );
        client.command("j", "SELECT INBOX").await;
        let coalesced = client.command("k", "UID MOVE 4 Trash").await;
        assert!(
            coalesced.last().unwrap().starts_with("k OK"),
            "{coalesced:?}"
        );
        assert!(
            coalesced.iter().all(|line| !line.contains("[COPYUID ")),
            "coalescing into canonical Trash must suppress COPYUID: {coalesced:?}"
        );
        drop(client);

        let mut client = Client::login(imap_address(&running), &issued).await;
        let inbox = client.command("o", "SELECT INBOX").await;
        assert!(inbox.contains(&"* OK [UIDNEXT 5] Predicted next UID\r\n".to_owned()));
        assert_eq!(
            client.command("p", "UID SEARCH ALL").await[0],
            "* SEARCH\r\n"
        );
        let trash = client.command("q", "SELECT Trash").await;
        assert!(trash.contains(&"* 3 EXISTS\r\n".to_owned()), "{trash:?}");
        assert!(trash.contains(&"* OK [UIDNEXT 4] Predicted next UID\r\n".to_owned()));
        let messages = client.command("r", "UID FETCH 1:3 (UID)").await;
        for uid in 1..=3 {
            assert!(
                messages
                    .iter()
                    .any(|line| line.contains(&format!("UID {uid}"))),
                "Trash must retain original UID {uid}: {messages:?}"
            );
        }
        drop(client);
        running.shutdown().await.unwrap();
    }
}

/// Concurrent Mirror COPYs create one shared membership, and Seen follows that copy across primary state.
#[tokio::test]
async fn concurrent_mirror_copy_is_atomic_and_seen_is_shared() {
    let (configuration, _directory) = configuration();
    let running = SandpostRuntime::start(configuration).await.unwrap();
    let owner = owner(&running).await;
    let caller = running.application.authorized(&owner);
    let first_credentials = create_account(
        &running,
        &owner,
        "Concurrent Mirror first",
        ImapAccountMode::Mirror,
    )
    .await;
    let second_credentials = create_account(
        &running,
        &owner,
        "Concurrent Mirror second",
        ImapAccountMode::Mirror,
    )
    .await;
    let independent_credentials = create_account(
        &running,
        &owner,
        "Copy independent",
        ImapAccountMode::Independent,
    )
    .await;
    send_mail(
        running.mail_listen_address,
        "dev@example.test",
        "Concurrent shared copy",
    )
    .await;
    wait_for_mail(&running, 1).await;
    let message = caller
        .query_web_mailbox(QueryOptions::default(), None)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let mut first = Client::login(imap_address(&running), &first_credentials).await;
    let mut second = Client::login(imap_address(&running), &second_credentials).await;
    let mut independent = Client::login(imap_address(&running), &independent_credentials).await;
    assert!(
        first
            .command("c", "CREATE Shared")
            .await
            .last()
            .unwrap()
            .starts_with("c OK")
    );
    first.command("s", "SELECT INBOX").await;
    second.command("s", "SELECT INBOX").await;

    let (first_copy, second_copy) = tokio::join!(
        first.command("a", "UID COPY 1 Shared"),
        second.command("b", "UID COPY 1 Shared"),
    );
    let responses = [first_copy, second_copy];
    assert_eq!(
        responses
            .iter()
            .filter(|response| response.last().unwrap().starts_with("a OK")
                || response.last().unwrap().starts_with("b OK"))
            .count(),
        1,
        "exactly one concurrent COPY must succeed: {responses:?}"
    );
    assert_eq!(
        responses
            .iter()
            .filter(|response| response.last().unwrap().contains("NO [CANNOT]"))
            .count(),
        1,
        "the duplicate destination COPY must return CANNOT: {responses:?}"
    );

    let folder = caller
        .list_web_folders()
        .await
        .unwrap()
        .into_iter()
        .find(|folder| folder.name == "Shared")
        .unwrap();
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

    first.command("f", "UID STORE 1 +FLAGS (\\Seen)").await;
    second.command("s", "SELECT Shared").await;
    assert!(
        second
            .command("f", "UID FETCH 1 (FLAGS)")
            .await
            .iter()
            .any(|line| line.contains("\\Seen"))
    );
    first.command("s", "SELECT INBOX").await;
    assert!(
        first
            .command("f", "UID FETCH 1 (FLAGS)")
            .await
            .iter()
            .any(|line| line.contains("\\Seen"))
    );
    independent.command("s", "SELECT INBOX").await;
    assert!(
        independent
            .command("f", "UID FETCH 1 (FLAGS)")
            .await
            .iter()
            .all(|line| !line.contains("\\Seen"))
    );
    running.shutdown().await.unwrap();
}

/// Subtree renames preserve child UID namespaces and LIST hierarchy in both organization modes.
#[tokio::test]
async fn hierarchy_subtree_rename_preserves_child_uids_in_both_modes() {
    let (configuration, _directory) = configuration();
    let running = SandpostRuntime::start(configuration).await.unwrap();
    let owner = owner(&running).await;
    send_mail(
        running.mail_listen_address,
        "dev@example.test",
        "Hierarchy identity",
    )
    .await;
    wait_for_mail(&running, 1).await;
    let mut obsolete_parent_listings = Vec::new();
    for (name, mode) in [
        ("Private hierarchy", ImapAccountMode::Independent),
        ("Primary hierarchy", ImapAccountMode::Mirror),
    ] {
        let issued = create_account(&running, &owner, name, mode).await;
        let mut client = Client::login(imap_address(&running), &issued).await;
        for (tag, command) in [("p", "CREATE Parent"), ("c", "CREATE Parent/Child")] {
            let response = client.command(tag, command).await;
            assert!(
                response.last().unwrap().starts_with(&format!("{tag} OK")),
                "{mode:?}: {response:?}"
            );
        }
        client.command("s", "SELECT INBOX").await;
        let copied = client.command("c", "UID COPY 1 Parent/Child").await;
        assert!(
            copied.last().unwrap().contains(" 1 1]"),
            "{mode:?}: {copied:?}"
        );
        let selected = client.command("s", "SELECT Parent/Child").await;
        let validity = uid_validity(&selected);
        let renamed = client.command("r", "RENAME Parent Archive").await;
        assert!(
            renamed.last().unwrap().starts_with("r OK"),
            "{mode:?}: {renamed:?}"
        );
        let selected = client.command("s", "SELECT Archive/Child").await;
        assert_eq!(uid_validity(&selected), validity);
        assert_eq!(
            client.command("q", "UID SEARCH ALL").await[0],
            "* SEARCH 1\r\n"
        );
        assert!(
            client
                .command("f", "UID FETCH 1 (UID BODY.PEEK[HEADER.FIELDS (SUBJECT)])")
                .await
                .iter()
                .any(|line| line.contains("UID 1") && line.contains("Hierarchy identity"))
        );
        let listings = client.command("l", "LIST \"\" *").await;
        assert!(
            listings
                .iter()
                .any(|line| line.contains("\\HasChildren") && line.ends_with(" Archive\r\n")),
            "{mode:?}: {listings:?}"
        );
        assert!(
            listings
                .iter()
                .any(|line| line.ends_with(" Archive/Child\r\n")),
            "{mode:?}: {listings:?}"
        );
        if listings.iter().any(|line| line.contains("Parent")) {
            obsolete_parent_listings.push((mode, listings));
        }
        let retained_subscriptions = client.command("ls", "LSUB \"\" *").await;
        assert!(
            retained_subscriptions
                .iter()
                .any(|line| line.contains("\\Noselect") && line.ends_with(" Parent/Child\r\n")),
            "{mode:?}: {retained_subscriptions:?}"
        );
        let old_name = client.command("old", "SELECT Parent/Child").await;
        assert!(
            old_name.last().unwrap().starts_with("old NO [NONEXISTENT]"),
            "{mode:?}: {old_name:?}"
        );
    }
    running.shutdown().await.unwrap();
    assert!(
        obsolete_parent_listings.is_empty(),
        "retained subscriptions belong only in LSUB: {obsolete_parent_listings:?}"
    );
}
