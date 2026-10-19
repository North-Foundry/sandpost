//! Primary and Independent message lifetime across restart.

use super::*;

/// Primary HTTP deletion and Independent expunge preserve other views and durable mailbox UIDs.
#[tokio::test]
async fn primary_deletion_and_independent_expunge_are_isolated_across_restart() {
    let (configuration, _directory) = configuration();
    let running = SandpostRuntime::start(configuration.clone()).await.unwrap();
    let owner_session = owner(&running).await;
    let owner_application = running.application.authorized(&owner_session);
    owner_application
        .create_user(sandpost_application::NewAccount {
            name: "Other reader".into(),
            email: "other@example.test".into(),
            password: "other test password".into(),
            global_role: GlobalRole::Member,
            mail_access: MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    let other = running
        .application
        .instance()
        .login(LoginRequest {
            remember_me: false,
            email: "other@example.test".into(),
            password: "other test password".into(),
        })
        .await
        .unwrap();
    let owner_login = running
        .application
        .instance()
        .login(LoginRequest {
            remember_me: false,
            email: "owner@example.test".into(),
            password: "runtime test password".into(),
        })
        .await
        .unwrap();
    let mut owner_accounts = Vec::new();
    for (name, mode) in [
        ("Primary", sandpost_core::ImapAccountMode::Mirror),
        ("First", sandpost_core::ImapAccountMode::Independent),
        ("Second", sandpost_core::ImapAccountMode::Independent),
    ] {
        owner_accounts.push(
            owner_application
                .create_imap_account(NewImapAccountRequest {
                    name: name.into(),
                    username: None,
                    mirror_views: true,
                    mode: Some(mode),
                    linked_views: Vec::new(),
                })
                .await
                .unwrap(),
        );
    }
    let other_account = running
        .application
        .authorized(&other.user)
        .create_imap_account(NewImapAccountRequest {
            name: "Other".into(),
            username: None,
            mirror_views: true,
            mode: Some(sandpost_core::ImapAccountMode::Independent),
            linked_views: Vec::new(),
        })
        .await
        .unwrap();
    send_mail(
        running.mail_listen_address,
        "a@example.test",
        "HTTP deletion",
    )
    .await;
    send_mail(
        running.mail_listen_address,
        "a@example.test",
        "IMAP deletion",
    )
    .await;
    wait_for_mail(&running, 2).await;
    running.application.wait_for_search().await.unwrap();
    let messages = owner_application.list_messages(None, 10).await.unwrap();
    let deleted_over_http = messages
        .iter()
        .find(|message| message.subject == "HTTP deletion")
        .unwrap()
        .identifier;
    let deleted_over_imap = messages
        .iter()
        .find(|message| message.subject == "IMAP deletion")
        .unwrap()
        .identifier;
    let mut primary = Client::login(imap_address(&running), &owner_accounts[0]).await;
    let mut first = Client::login(imap_address(&running), &owner_accounts[1]).await;
    let mut second = Client::login(imap_address(&running), &owner_accounts[2]).await;
    let mut other_client = Client::login(imap_address(&running), &other_account).await;
    let primary_validity = uid_validity(&primary.command("s", "SELECT INBOX").await);
    let first_validity = uid_validity(&first.command("s", "SELECT INBOX").await);
    let second_validity = uid_validity(&second.command("s", "SELECT INBOX").await);
    other_client.command("s", "SELECT INBOX").await;
    for client in [&mut primary, &mut first, &mut second, &mut other_client] {
        assert_eq!(
            client.command("q", "UID SEARCH ALL").await[0],
            "* SEARCH 1 2\r\n"
        );
    }
    for client in [&mut primary, &mut second] {
        client.0.get_mut().write_all(b"i IDLE\r\n").await.unwrap();
        assert_eq!(client.response().await, "+ idling\r\n");
    }
    let response = http_request(
        &running,
        &owner_login.token,
        "DELETE",
        &format!("/api/v1/messages/{deleted_over_http}"),
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 204"), "{response}");
    assert_eq!(primary.response().await, "* 1 EXPUNGE\r\n");
    for client in [&mut primary, &mut second] {
        client.0.get_mut().write_all(b"DONE\r\n").await.unwrap();
        assert_eq!(client.response().await, "i OK IDLE terminated\r\n");
    }
    assert_eq!(
        primary.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 2\r\n"
    );
    for client in [&mut first, &mut second, &mut other_client] {
        assert_eq!(
            client.command("q", "UID SEARCH ALL").await[0],
            "* SEARCH 1 2\r\n"
        );
    }
    let retained = first.command("f", "UID FETCH 1 (UID BODY.PEEK[])").await;
    assert!(
        retained.iter().any(|line| line.contains("HTTP deletion")),
        "{retained:?}"
    );
    for command in [
        "UID STORE 2 +FLAGS (\\Deleted)",
        "UID STORE 2 -FLAGS (\\Deleted)",
        "UID STORE 2 +FLAGS (\\Deleted)",
    ] {
        let response = first.command("f", command).await;
        assert!(response.last().unwrap().starts_with("f OK"), "{response:?}");
        assert!(
            owner_application
                .get_message(deleted_over_imap)
                .await
                .is_ok()
        );
    }
    let expunge = first.command("e", "UID EXPUNGE 2").await;
    assert!(
        expunge.contains(&"* 2 EXPUNGE\r\n".to_owned()),
        "{expunge:?}"
    );
    assert_eq!(
        first.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 1\r\n"
    );
    assert_eq!(
        second.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 1 2\r\n"
    );
    let trash = first.command("t", "SELECT Trash").await;
    assert!(trash.contains(&"* 1 EXISTS\r\n".to_owned()), "{trash:?}");
    assert_eq!(
        first.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 1\r\n"
    );
    first.command("d", "UID STORE 1 +FLAGS (\\Deleted)").await;
    let released = first.command("e", "UID EXPUNGE 1").await;
    assert!(
        released.contains(&"* 1 EXPUNGE\r\n".to_owned()),
        "{released:?}"
    );
    assert!(
        http_request(
            &running,
            &owner_login.token,
            "GET",
            &format!("/api/v1/messages/{deleted_over_http}")
        )
        .await
        .starts_with("HTTP/1.1 404")
    );
    assert!(
        http_request(
            &running,
            &owner_login.token,
            "GET",
            &format!("/api/v1/messages/{deleted_over_imap}")
        )
        .await
        .starts_with("HTTP/1.1 200")
    );
    assert!(
        http_request(
            &running,
            &other.token,
            "GET",
            &format!("/api/v1/messages/{deleted_over_imap}/raw")
        )
        .await
        .starts_with("HTTP/1.1 200")
    );
    assert_eq!(
        other_client.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 1 2\r\n"
    );
    drop(primary);
    drop(first);
    drop(second);
    drop(other_client);
    running.shutdown().await.unwrap();
    let restarted = SandpostRuntime::start(configuration).await.unwrap();
    for (credential, validity, expected) in [
        (&owner_accounts[0], &primary_validity, "* SEARCH 2\r\n"),
        (&owner_accounts[1], &first_validity, "* SEARCH 1\r\n"),
        (&owner_accounts[2], &second_validity, "* SEARCH 1 2\r\n"),
    ] {
        let mut client = Client::login(imap_address(&restarted), credential).await;
        assert_eq!(
            &uid_validity(&client.command("s", "SELECT INBOX").await),
            validity
        );
        assert_eq!(client.command("q", "UID SEARCH ALL").await[0], expected);
    }
    let mut other_client = Client::login(imap_address(&restarted), &other_account).await;
    other_client.command("s", "SELECT INBOX").await;
    assert_eq!(
        other_client.command("q", "UID SEARCH ALL").await[0],
        "* SEARCH 1 2\r\n"
    );
    restarted.shutdown().await.unwrap();
}
