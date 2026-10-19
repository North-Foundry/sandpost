//! Live permission and credential revocation through IMAP IDLE.

use super::*;

/// A scoped member's IMAP account shows only assigned mail, and an administrator revoking the
/// scope removes it from an idling client immediately.
#[tokio::test]
async fn revoking_a_scope_expunges_from_idle_clients() {
    let (configuration, _directory) = configuration();
    let running = SandpostRuntime::start(configuration).await.unwrap();
    let owner = owner(&running).await;
    let administrator = running.application.authorized(&owner);
    let member = administrator
        .create_user(sandpost_application::NewAccount {
            name: "Member".into(),
            email: "member@example.test".into(),
            password: "member test password".into(),
            global_role: GlobalRole::Member,
            mail_access: MailAccess::Scoped,
            personal_filter: None,
        })
        .await
        .unwrap();
    let scope = administrator
        .save_scope(Scope {
            identifier: ScopeIdentifier::new(),
            parent: None,
            name: "Team".into(),
            description: None,
            filter: "envelope.to.domain == \"team.test\"".into(),
            position: 0,
            policy_version: 0,
        })
        .await
        .unwrap();
    administrator
        .assign_scope(member.identifier, scope.identifier)
        .await
        .unwrap();
    let member_session = running
        .application
        .instance()
        .login(LoginRequest {
            remember_me: false,
            email: "member@example.test".into(),
            password: "member test password".into(),
        })
        .await
        .unwrap()
        .user;
    let issued = running
        .application
        .authorized(&member_session)
        .create_imap_account(NewImapAccountRequest {
            name: "Team".into(),
            username: None,
            mirror_views: true,
            mode: Some(sandpost_core::ImapAccountMode::Independent),
            linked_views: Vec::new(),
        })
        .await
        .unwrap();
    let smtp = running.mail_listen_address;
    send_mail(smtp, "dev@team.test", "Team mail").await;
    send_mail(smtp, "dev@private.test", "Private mail").await;
    wait_for_mail(&running, 2).await;
    let mut client = Client::login(imap_address(&running), &issued).await;
    let selected = client.command("s", "SELECT INBOX").await;
    assert!(
        selected.contains(&"* 1 EXISTS\r\n".to_owned()),
        "{selected:?}"
    );
    let all = client.command("q", "SEARCH TEXT \"Private\"").await;
    assert_eq!(
        all[0], "* SEARCH\r\n",
        "unauthorized mail is never searchable"
    );
    let direct = client.command("f", "UID FETCH 2 (UID BODY.PEEK[])").await;
    assert_eq!(
        direct,
        ["f OK FETCH completed\r\n"],
        "no UID reaches unauthorized mail"
    );
    client.0.get_mut().write_all(b"i IDLE\r\n").await.unwrap();
    assert_eq!(client.response().await, "+ idling\r\n");
    administrator
        .remove_scope_assignment(member.identifier, scope.identifier)
        .await
        .unwrap();
    assert_eq!(client.response().await, "* 1 EXPUNGE\r\n");
    client.0.get_mut().write_all(b"DONE\r\n").await.unwrap();
    assert_eq!(client.response().await, "i OK IDLE terminated\r\n");
    running.shutdown().await.unwrap();
}

/// Disabling an account from the management API disconnects its idling clients.
#[tokio::test]
async fn disabling_an_account_disconnects_idle_clients() {
    let (configuration, _directory) = configuration();
    let running = SandpostRuntime::start(configuration).await.unwrap();
    let owner = owner(&running).await;
    let caller = running.application.authorized(&owner);
    let issued = caller
        .create_imap_account(NewImapAccountRequest {
            name: "Revocable".into(),
            username: None,
            mirror_views: true,
            mode: Some(sandpost_core::ImapAccountMode::Independent),
            linked_views: Vec::new(),
        })
        .await
        .unwrap();
    let mut client = Client::login(imap_address(&running), &issued).await;
    client.command("s", "SELECT INBOX").await;
    client.0.get_mut().write_all(b"i IDLE\r\n").await.unwrap();
    assert_eq!(client.response().await, "+ idling\r\n");
    caller
        .update_imap_account(
            issued.account.account.identifier,
            sandpost_application::ImapAccountUpdate {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        client.response().await,
        "* BYE This IMAP account is no longer available\r\n"
    );
    running.shutdown().await.unwrap();
}

/// Rotating credentials revokes existing IDLE sessions and permits only the new password.
#[tokio::test]
async fn rotating_password_revokes_idle_sessions_and_old_credentials() {
    let (configuration, _directory) = configuration();
    let running = SandpostRuntime::start(configuration).await.unwrap();
    let owner = owner(&running).await;
    let caller = running.application.authorized(&owner);
    let issued = caller
        .create_imap_account(NewImapAccountRequest {
            name: "Rotatable".into(),
            username: None,
            mirror_views: true,
            mode: Some(sandpost_core::ImapAccountMode::Mirror),
            linked_views: Vec::new(),
        })
        .await
        .unwrap();
    let address = imap_address(&running);
    let mut existing = Client::login(address, &issued).await;
    existing.command("s", "SELECT INBOX").await;
    existing.0.get_mut().write_all(b"i IDLE\r\n").await.unwrap();
    assert_eq!(existing.response().await, "+ idling\r\n");
    let rotated = caller
        .regenerate_imap_account_password(issued.account.account.identifier)
        .await
        .unwrap();
    assert_eq!(
        existing.response().await,
        "* BYE This IMAP account is no longer available\r\n"
    );
    let mut old_credentials = Client::connect(address).await;
    let rejected = old_credentials
        .command(
            "l",
            &format!(
                "LOGIN {} \"{}\"",
                issued.account.account.username, issued.password
            ),
        )
        .await;
    assert!(rejected.last().unwrap().starts_with("l NO"), "{rejected:?}");
    let mut replacement = Client::login(address, &rotated).await;
    let selected = replacement.command("s", "SELECT INBOX").await;
    assert!(selected.last().unwrap().starts_with("s OK"), "{selected:?}");
    running.shutdown().await.unwrap();
}
