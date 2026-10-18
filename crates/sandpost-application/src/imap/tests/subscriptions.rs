//! Subscription retention across catalog changes without granting mailbox access.
use super::*;
use crate::WebFolderRequest;

/// Observed defaults survive deletion, while listing never overrides unsubscribe exceptions.
#[tokio::test]
async fn observed_default_subscriptions_survive_deletion_without_resubscribing() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    let issued = fixture.account(&user, ImapAccountMode::Independent).await;
    let session = fixture.login(&issued).await;
    let account = issued.account.account.identifier;
    session.create_mailbox("Default").await.unwrap();
    session.create_mailbox("Ignored").await.unwrap();
    session.set_subscription("Ignored", false).await.unwrap();

    let listings = session.mailboxes().await.unwrap();
    assert!(
        listings.iter().any(|mailbox| {
            mailbox.name == "Default" && mailbox.selectable && mailbox.subscribed
        })
    );
    assert!(
        listings.iter().any(|mailbox| {
            mailbox.name == "Ignored" && mailbox.selectable && !mailbox.subscribed
        })
    );
    let retained = fixture.storage.list_imap_subscribed(account).await.unwrap();
    assert_eq!(retained, ["Default", "INBOX", "Trash"]);
    assert_eq!(session.mailboxes().await.unwrap(), listings);
    assert_eq!(
        fixture.storage.list_imap_subscribed(account).await.unwrap(),
        retained
    );
    assert_eq!(
        fixture
            .storage
            .list_imap_unsubscribed(account)
            .await
            .unwrap(),
        ["Ignored"]
    );

    session.delete_mailbox("Default").await.unwrap();
    session.delete_mailbox("Ignored").await.unwrap();
    let listings = session.mailboxes().await.unwrap();
    let deleted = listings
        .iter()
        .find(|mailbox| mailbox.name == "Default")
        .unwrap();
    assert!(deleted.subscribed);
    assert!(!deleted.selectable);
    assert!(!listings.iter().any(|mailbox| mailbox.name == "Ignored"));
    assert!(matches!(
        session.open("Default", false).await,
        Err(ImapAccessError::NonExistent)
    ));

    let next_session = fixture.login(&issued).await;
    assert_eq!(next_session.mailboxes().await.unwrap(), listings);
    next_session
        .set_subscription("Default", false)
        .await
        .unwrap();
    assert!(
        !next_session
            .mailboxes()
            .await
            .unwrap()
            .iter()
            .any(|mailbox| mailbox.name == "Default")
    );
}

/// Subscriptions survive deletion and rename only for the subscribing account.
#[tokio::test]
async fn retained_subscriptions_are_not_selectable_until_the_mailbox_returns() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    let first = fixture.account(&user, ImapAccountMode::Mirror).await;
    let second = fixture.account(&user, ImapAccountMode::Mirror).await;
    let session = fixture.login(&first).await;
    let second_session = fixture.login(&second).await;
    assert!(
        session
            .mailboxes()
            .await
            .unwrap()
            .iter()
            .all(|mailbox| { mailbox.selectable && mailbox.subscribed })
    );

    session.set_subscription("INBOX", false).await.unwrap();
    session.set_subscription("Missing", true).await.unwrap();
    let listings = session.mailboxes().await.unwrap();
    let inbox = listings
        .iter()
        .find(|mailbox| mailbox.name == "INBOX")
        .unwrap();
    assert!(inbox.selectable);
    assert!(!inbox.subscribed);
    let missing = listings
        .iter()
        .find(|mailbox| mailbox.name == "Missing")
        .unwrap();
    assert!(missing.subscribed);
    assert!(!missing.selectable);
    assert!(!missing.is_trash);
    assert!(matches!(
        session.open("Missing", false).await,
        Err(ImapAccessError::NonExistent)
    ));

    session.create_mailbox("Private").await.unwrap();
    session.set_subscription("Private", true).await.unwrap();
    session.delete_mailbox("Private").await.unwrap();
    session.create_mailbox("Before").await.unwrap();
    session.set_subscription("Before", true).await.unwrap();
    session.rename_mailbox("Before", "After").await.unwrap();
    let caller = fixture.as_user(&user);
    let folder = caller
        .create_web_folder(WebFolderRequest {
            name: "Exposed".into(),
            icon: None,
            exposed_accounts: vec![first.account.account.identifier],
        })
        .await
        .unwrap();
    session.set_subscription("Exposed", true).await.unwrap();
    // Mirror always follows primary folders, regardless of the legacy exposure list.
    assert!(second_session.open("Exposed", true).await.is_ok());
    caller.delete_web_folder(folder.identifier).await.unwrap();

    let listings = session.mailboxes().await.unwrap();
    for name in ["Missing", "Private", "Before", "Exposed"] {
        let retained = listings
            .iter()
            .find(|mailbox| mailbox.name == name)
            .unwrap();
        assert!(retained.subscribed);
        assert!(!retained.selectable);
        assert!(matches!(
            session.open(name, false).await,
            Err(ImapAccessError::NonExistent)
        ));
    }
    let renamed = listings
        .iter()
        .find(|mailbox| mailbox.name == "After")
        .unwrap();
    assert!(renamed.selectable && renamed.subscribed);
    assert!(
        second_session
            .mailboxes()
            .await
            .unwrap()
            .iter()
            .all(|mailbox| { mailbox.selectable && mailbox.subscribed })
    );

    session.set_subscription("Missing", false).await.unwrap();
    assert!(
        !session
            .mailboxes()
            .await
            .unwrap()
            .iter()
            .any(|mailbox| mailbox.name == "Missing")
    );
    session.create_mailbox("Private").await.unwrap();
    let listings = session.mailboxes().await.unwrap();
    let returned: Vec<_> = listings
        .iter()
        .filter(|mailbox| mailbox.name == "Private")
        .collect();
    assert_eq!(returned.len(), 1);
    assert!(returned[0].selectable && returned[0].subscribed);
    session.set_subscription("Private", false).await.unwrap();
    session.delete_mailbox("Private").await.unwrap();
    assert!(
        !session
            .mailboxes()
            .await
            .unwrap()
            .iter()
            .any(|mailbox| mailbox.name == "Private")
    );
}
