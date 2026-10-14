//! Restricted smart-mailbox operations over the real protocol.

use super::*;

/// Smart mailboxes permit reads and `\\Seen` updates while refusing membership changes.
#[tokio::test]
async fn smart_mailbox_protocol_rights_are_seen_only() {
    let (configuration, _directory) = configuration();
    let running = SandpostRuntime::start(configuration).await.unwrap();
    let owner = owner(&running).await;
    let administrator = running.application.authorized(&owner);
    let issued = administrator
        .create_imap_account(NewImapAccountRequest {
            name: "Smart rights".into(),
            username: None,
            mirror_views: true,
            mode: Some(sandpost_core::ImapAccountMode::Independent),
            linked_views: Vec::new(),
        })
        .await
        .unwrap();
    administrator
        .create_imap_folder(
            issued.account.account.identifier,
            ImapFolderRequest {
                name: "Smart".into(),
                filter: "subject contains \"Smart rights\"".into(),
                include_in_inbox: false,
            },
        )
        .await
        .unwrap();
    send_mail(
        running.mail_listen_address.unwrap(),
        "dev@example.test",
        "Smart rights message",
    )
    .await;
    wait_for_mail(&running, 1).await;

    let mut client = Client::login(imap_address(&running), &issued).await;
    client.command("c", "CREATE Archive").await;
    let delete = client.command("z", "DELETE Smart").await;
    assert!(
        delete.last().unwrap().starts_with("z NO [CANNOT]"),
        "{delete:?}"
    );
    client.command("i", "SELECT INBOX").await;
    assert!(
        client
            .command("t", "STORE 1 +FLAGS (\\Flagged)")
            .await
            .last()
            .unwrap()
            .starts_with("t OK")
    );
    let selected = client.command("s", "SELECT Smart").await;
    assert!(
        selected
            .iter()
            .any(|line| line.starts_with("* FLAGS ") && line.contains("\\Flagged")),
        "{selected:?}"
    );
    assert!(
        selected
            .iter()
            .any(|line| line.contains("[PERMANENTFLAGS (\\Seen)]"))
    );
    assert!(selected.last().unwrap().contains("[READ-WRITE]"));
    assert!(
        client
            .command("v", "STORE 1 +FLAGS (\\Seen)")
            .await
            .last()
            .unwrap()
            .starts_with("v OK")
    );
    assert!(
        client
            .command("w", "STORE 1 FLAGS ()")
            .await
            .last()
            .unwrap()
            .starts_with("w OK")
    );
    let cleared = client.command("wf", "UID FETCH 1 (FLAGS)").await;
    assert!(
        cleared
            .iter()
            .any(|line| line.contains("\\Flagged") && !line.contains("\\Seen")),
        "{cleared:?}"
    );
    assert!(
        client
            .command("y", "STORE 1 FLAGS (\\Seen)")
            .await
            .last()
            .unwrap()
            .starts_with("y OK")
    );
    let replaced = client.command("yf", "UID FETCH 1 (FLAGS)").await;
    assert!(
        replaced
            .iter()
            .any(|line| line.contains("\\Flagged") && line.contains("\\Seen")),
        "{replaced:?}"
    );
    for (tag, command) in [
        ("f", "STORE 1 +FLAGS (\\Flagged)"),
        ("an", "UID STORE 1 +FLAGS (\\Answered)"),
        ("dr", "STORE 1 -FLAGS (\\Draft)"),
        ("kw", "STORE 1 FLAGS (\\Seen Work)"),
        ("d", "STORE 1 +FLAGS (\\Deleted)"),
        ("e", "EXPUNGE"),
        ("ue", "UID EXPUNGE 1"),
        ("m", "MOVE 1 Archive"),
        ("um", "UID MOVE 1 Archive"),
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
    assert!(
        client
            .command("o", "COPY 1 Archive")
            .await
            .last()
            .unwrap()
            .starts_with("o OK")
    );
    assert_eq!(
        client.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 1\r\n",
        "COPY must preserve smart membership"
    );
    assert!(
        client
            .command("x", "CLOSE")
            .await
            .last()
            .unwrap()
            .starts_with("x OK")
    );
    assert!(
        client
            .command("h", "SELECT INBOX")
            .await
            .contains(&"* 1 EXISTS\r\n".to_owned())
    );

    client.command("a", "SELECT Archive").await;
    let move_into_smart = client.command("n", "MOVE 1 Smart").await;
    assert!(
        move_into_smart.last().unwrap().starts_with("n NO [CANNOT]"),
        "{move_into_smart:?}"
    );
    let copy_into_smart = client.command("cp", "UID COPY 1 Smart").await;
    assert!(
        copy_into_smart
            .last()
            .unwrap()
            .starts_with("cp NO [CANNOT]"),
        "{copy_into_smart:?}"
    );
    client.command("s2", "SELECT Smart").await;
    client.command("cl", "STORE 1 FLAGS ()").await;
    let examine = client.command("r", "EXAMINE Smart").await;
    assert!(
        examine
            .iter()
            .any(|line| line.contains("[PERMANENTFLAGS ()]"))
    );
    assert!(examine.last().unwrap().contains("[READ-ONLY]"));
    let examined_store = client.command("ro", "STORE 1 +FLAGS (\\Seen)").await;
    assert!(
        examined_store.last().unwrap().starts_with("ro NO"),
        "{examined_store:?}"
    );
    let examined_fetch = client.command("rf", "UID FETCH 1 (FLAGS BODY[])").await;
    assert!(
        examined_fetch
            .iter()
            .any(|line| line.contains("\\Flagged") && !line.contains("\\Seen")),
        "{examined_fetch:?}"
    );
    let append = client.command("p", "APPEND Smart {0+}\r\n").await;
    assert!(
        append.last().unwrap().starts_with("p NO [CANNOT]"),
        "{append:?}"
    );
    running.shutdown().await.unwrap();
}
