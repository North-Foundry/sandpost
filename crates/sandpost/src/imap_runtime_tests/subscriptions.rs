//! Real-command IMAP tests for durable subscriptions to absent mailbox names.
use super::*;

/// Explicit subscriptions survive private mailbox deletion, while LIST omits retained ghosts.
#[tokio::test]
async fn deleted_mailbox_subscription_is_lsub_only_noselect() {
    let (configuration, _directory) = configuration();
    let running = SandpostRuntime::start(configuration).await.unwrap();
    let owner_session = owner(&running).await;
    let issued = running
        .application
        .authorized(&owner_session)
        .create_imap_account(NewImapAccountRequest {
            name: "Subscriptions".into(),
            username: None,
            mirror_views: true,
            mode: Some(sandpost_core::ImapAccountMode::Independent),
            linked_views: Vec::new(),
        })
        .await
        .unwrap();
    let mut client = Client::login(imap_address(&running), &issued).await;

    let created_default = client.command("cd", "CREATE Default").await;
    assert!(
        created_default.last().unwrap().starts_with("cd OK"),
        "{created_default:?}"
    );
    let discovery = client.command("ld", "LIST \"\" *").await;
    assert!(
        discovery.iter().any(|line| line.ends_with(" Default\r\n")),
        "LIST discovers default-subscribed mailbox: {discovery:?}"
    );
    let deleted_default = client.command("dd", "DELETE Default").await;
    assert!(
        deleted_default.last().unwrap().starts_with("dd OK"),
        "{deleted_default:?}"
    );
    let no_default_ghost_in_list = client.command("ld2", "LIST \"\" *").await;
    assert!(
        no_default_ghost_in_list
            .iter()
            .all(|line| !line.contains("Default")),
        "LIST omits observed default ghost: {no_default_ghost_in_list:?}"
    );
    let default_ghost = client.command("lsd", "LSUB \"\" *").await;
    assert!(
        default_ghost
            .iter()
            .any(|line| line == "* LSUB (\\Noselect) \"/\" Default\r\n"),
        "observed default remains a Noselect ghost: {default_ghost:?}"
    );

    let created = client.command("c", "CREATE Private").await;
    assert!(created.last().unwrap().starts_with("c OK"), "{created:?}");
    let subscribed = client.command("s", "SUBSCRIBE Private").await;
    assert!(
        subscribed.last().unwrap().starts_with("s OK"),
        "{subscribed:?}"
    );
    let deleted = client.command("d", "DELETE Private").await;
    assert!(deleted.last().unwrap().starts_with("d OK"), "{deleted:?}");

    let listed = client.command("l", "LIST \"\" *").await;
    assert!(
        listed.iter().all(|line| !line.contains("Private")),
        "LIST must omit absent subscribed names: {listed:?}"
    );
    let subscribed = client.command("ls", "LSUB \"\" *").await;
    assert!(
        subscribed
            .iter()
            .any(|line| line == "* LSUB (\\Noselect) \"/\" Private\r\n"),
        "LSUB retains the absent name as nonselectable: {subscribed:?}"
    );

    let recreated = client.command("c2", "CREATE Private").await;
    assert!(
        recreated.last().unwrap().starts_with("c2 OK"),
        "{recreated:?}"
    );
    let subscribed = client.command("ls2", "LSUB \"\" *").await;
    assert!(
        subscribed
            .iter()
            .any(|line| line == "* LSUB (\\HasNoChildren) \"/\" Private\r\n"),
        "the name becomes selectable again when recreated: {subscribed:?}"
    );

    client.command("d2", "DELETE Private").await;
    client.command("u", "UNSUBSCRIBE Private").await;
    let subscribed = client.command("ls3", "LSUB \"\" *").await;
    assert!(
        subscribed.iter().all(|line| !line.contains("Private")),
        "UNSUBSCRIBE removes the retained name: {subscribed:?}"
    );
    running.shutdown().await.unwrap();
}
