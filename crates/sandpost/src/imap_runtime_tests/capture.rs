//! SMTP capture, IMAP IDLE delivery, and restart persistence.

use super::*;

/// SMTP mail reaches an idling IMAP client without polling; FETCH and SEARCH read it; UIDs
/// survive a restart.
#[tokio::test]
async fn smtp_mail_is_pushed_to_idle_clients_and_uids_persist() {
    let (configuration, _directory) = configuration();
    let running = SandpostRuntime::start(configuration.clone()).await.unwrap();
    let owner = owner(&running).await;
    let issued = running
        .application
        .authorized(&owner)
        .create_imap_account(NewImapAccountRequest {
            name: "Laravel Debug".into(),
            username: None,
            mirror_views: true,
            mode: Some(sandpost_core::ImapAccountMode::Independent),
            linked_views: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(issued.server.port, Some(imap_address(&running).port()));
    let smtp = running.mail_listen_address.unwrap();
    send_mail(smtp, "dev@example.test", "First test mail").await;
    wait_for_mail(&running, 1).await;
    let mut client = Client::login(imap_address(&running), &issued).await;
    let selected = client.command("s", "SELECT INBOX").await;
    assert!(
        selected.contains(&"* 1 EXISTS\r\n".to_owned()),
        "{selected:?}"
    );
    let validity = selected
        .iter()
        .find_map(|line| line.strip_prefix("* OK [UIDVALIDITY "))
        .and_then(|rest| rest.split(']').next())
        .unwrap()
        .to_owned();
    client.0.get_mut().write_all(b"i IDLE\r\n").await.unwrap();
    assert_eq!(client.response().await, "+ idling\r\n");
    send_mail(smtp, "dev@example.test", "Second test mail").await;
    assert_eq!(client.response().await, "* 2 EXISTS\r\n");
    assert_eq!(client.response().await, "* 2 RECENT\r\n");
    client.0.get_mut().write_all(b"DONE\r\n").await.unwrap();
    assert_eq!(client.response().await, "i OK IDLE terminated\r\n");
    let fetched = client
        .command(
            "f",
            "UID FETCH 1:* (UID ENVELOPE BODY.PEEK[HEADER.FIELDS (SUBJECT)])",
        )
        .await;
    assert!(
        fetched[0].contains("UID 1 ") && fetched[0].contains("First test mail"),
        "{fetched:?}"
    );
    assert!(
        fetched[1].contains("UID 2 ") && fetched[1].contains("Second test mail"),
        "{fetched:?}"
    );
    let searched = client.command("q", "UID SEARCH SUBJECT second").await;
    assert_eq!(searched[0], "* SEARCH 2\r\n");
    running.shutdown().await.unwrap();

    let restarted = SandpostRuntime::start(configuration).await.unwrap();
    let mut client = Client::login(imap_address(&restarted), &issued).await;
    let selected = client.command("s", "EXAMINE INBOX").await;
    assert!(
        selected.contains(&format!("* OK [UIDVALIDITY {validity}] UIDs valid\r\n")),
        "UIDVALIDITY survives restart: {selected:?}"
    );
    let fetched = client.command("f", "FETCH 1:* (UID)").await;
    assert_eq!(
        fetched[..2],
        ["* 1 FETCH (UID 1)\r\n", "* 2 FETCH (UID 2)\r\n"]
    );
    restarted.shutdown().await.unwrap();
}
