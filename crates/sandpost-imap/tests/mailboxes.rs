//! Mailbox command conformance (RFC 3501 section 6.3, RFC 3348 CHILDREN, RFC 6154 attributes,
//! RFC 3691 UNSELECT, modified UTF-7 names).
mod support;

use support::{RunningServer, plain_message, test_limits};

/// LIST returns the delimiter for an empty pattern and every matching mailbox with attributes.
#[tokio::test]
async fn list_patterns_and_attributes() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.connect().await;
    client.login().await;
    assert_eq!(
        client.command("l1", "LIST \"\" \"\"").await,
        [
            "* LIST (\\Noselect) \"/\" \"\"\r\n",
            "l1 OK LIST completed\r\n"
        ]
    );
    assert_eq!(
        client.command("l2", "LIST \"\" *").await,
        [
            "* LIST (\\HasNoChildren) \"/\" Caf&AOk-\r\n",
            "* LIST (\\HasNoChildren) \"/\" INBOX\r\n",
            "* LIST (\\HasNoChildren) \"/\" Laravel\r\n",
            "* LIST (\\HasNoChildren \\Trash) \"/\" Trash\r\n",
            "l2 OK LIST completed\r\n"
        ]
    );
    assert_eq!(
        client.command("l3", "LIST \"\" inbox").await,
        [
            "* LIST (\\HasNoChildren) \"/\" INBOX\r\n",
            "l3 OK LIST completed\r\n"
        ]
    );
    assert_eq!(
        client.command("l4", "LIST \"\" T%").await,
        [
            "* LIST (\\HasNoChildren \\Trash) \"/\" Trash\r\n",
            "l4 OK LIST completed\r\n"
        ]
    );
    assert_eq!(
        client.command("l5", "LIST \"\" Nothing*").await,
        ["l5 OK LIST completed\r\n"]
    );
    server.stop().await;
}

/// SUBSCRIBE and UNSUBSCRIBE change what LSUB returns.
#[tokio::test]
async fn subscriptions_drive_lsub() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.connect().await;
    client.login().await;
    assert_eq!(
        client.status_of("s1", "UNSUBSCRIBE Laravel").await,
        "s1 OK UNSUBSCRIBE completed\r\n"
    );
    let listed = client.command("s2", "LSUB \"\" *").await;
    assert!(
        listed.iter().all(|line| !line.contains("Laravel")),
        "{listed:?}"
    );
    assert!(listed.iter().any(|line| line.contains("INBOX")));
    client.status_of("s3", "SUBSCRIBE Laravel").await;
    let listed = client.command("s4", "LSUB \"\" *").await;
    assert!(listed.iter().any(|line| line.contains("Laravel")));
    server.stop().await;
}

/// STATUS reports every requested item, decoding modified UTF-7 names.
#[tokio::test]
async fn status_reports_counts() {
    let server = RunningServer::start(test_limits()).await;
    server
        .backend
        .deliver("Caf\u{e9}", &plain_message("one"), 0);
    server
        .backend
        .deliver("Caf\u{e9}", &plain_message("two"), 0);
    let mut client = server.connect().await;
    client.login().await;
    assert_eq!(
        client
            .command(
                "s1",
                "STATUS Caf&AOk- (MESSAGES RECENT UIDNEXT UIDVALIDITY UNSEEN)"
            )
            .await,
        [
            "* STATUS Caf&AOk- (MESSAGES 2 RECENT 2 UIDNEXT 3 UIDVALIDITY 1003 UNSEEN 2)\r\n",
            "s1 OK STATUS completed\r\n"
        ]
    );
    assert_eq!(
        client.status_of("s2", "STATUS Missing (MESSAGES)").await,
        "s2 NO [NONEXISTENT] Mailbox does not exist\r\n"
    );
    server.stop().await;
}

/// SELECT returns the required untagged data and READ-WRITE; EXAMINE is READ-ONLY with empty
/// PERMANENTFLAGS; a failed SELECT leaves no mailbox selected (RFC 3501 section 6.3.1).
#[tokio::test]
async fn select_and_examine() {
    let server = RunningServer::start(test_limits()).await;
    server.backend.deliver("INBOX", &plain_message("one"), 0);
    let mut client = server.connect().await;
    client.login().await;
    assert_eq!(
        client.command("s1", "SELECT INBOX").await,
        [
            "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n",
            "* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft \\*)] Flags permitted\r\n",
            "* 1 EXISTS\r\n",
            "* 1 RECENT\r\n",
            "* OK [UNSEEN 1] First unseen message\r\n",
            "* OK [UIDVALIDITY 1000] UIDs valid\r\n",
            "* OK [UIDNEXT 2] Predicted next UID\r\n",
            "s1 OK [READ-WRITE] SELECT completed\r\n"
        ]
    );
    let examined = client.command("s2", "EXAMINE INBOX").await;
    assert!(
        examined.contains(&"* OK [UIDNEXT 2] Predicted next UID\r\n".to_owned()),
        "EXAMINE must report the next UID just like SELECT: {examined:?}"
    );
    assert_eq!(examined[1], "* OK [PERMANENTFLAGS ()] Flags permitted\r\n");
    assert!(
        examined.contains(&"* 0 RECENT\r\n".to_owned()),
        "already claimed: {examined:?}"
    );
    assert_eq!(
        examined.last().unwrap(),
        "s2 OK [READ-ONLY] EXAMINE completed\r\n"
    );
    assert_eq!(
        client.status_of("s3", "SELECT Missing").await,
        "s3 NO [NONEXISTENT] Mailbox does not exist\r\n"
    );
    assert!(
        client
            .status_of("s4", "FETCH 1 FLAGS")
            .await
            .starts_with("s4 BAD"),
        "the failed SELECT closed the previous mailbox"
    );
    server.stop().await;
}

/// UNSELECT and CLOSE return to the authenticated state; CLOSE expunges silently.
#[tokio::test]
async fn unselect_and_close() {
    let server = RunningServer::start(test_limits()).await;
    let uid = server.backend.deliver("INBOX", &plain_message("one"), 0);
    let mut client = server.selected("INBOX").await;
    client
        .status_of("u1", "STORE 1 +FLAGS.SILENT (\\Deleted)")
        .await;
    assert_eq!(
        client.command("u2", "UNSELECT").await,
        ["u2 OK UNSELECT completed\r\n"]
    );
    assert_eq!(
        server.backend.uids("INBOX"),
        [uid],
        "UNSELECT does not expunge"
    );
    client.status_of("u3", "SELECT INBOX").await;
    assert_eq!(
        client.command("u4", "CLOSE").await,
        ["u4 OK CLOSE completed\r\n"],
        "no untagged EXPUNGE"
    );
    assert!(server.backend.uids("INBOX").is_empty());
    assert!(client.status_of("u5", "CLOSE").await.starts_with("u5 BAD"));
    server.stop().await;
}

/// Mailbox management is refused with RFC 5530 codes when the backend does not support it.
#[tokio::test]
async fn create_delete_rename_are_refused_validly() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.connect().await;
    client.login().await;
    for (command, expected) in [
        (
            "CREATE Archive",
            "m1 NO [CANNOT] mailboxes cannot be created through IMAP\r\n",
        ),
        (
            "CREATE INBOX",
            "m1 NO [ALREADYEXISTS] Mailbox already exists\r\n",
        ),
        (
            "CREATE Laravel",
            "m1 NO [ALREADYEXISTS] Mailbox already exists\r\n",
        ),
        ("DELETE INBOX", "m1 NO [CANNOT] INBOX cannot be deleted\r\n"),
        (
            "DELETE Missing",
            "m1 NO [NONEXISTENT] Mailbox does not exist\r\n",
        ),
        (
            "DELETE Laravel",
            "m1 NO [CANNOT] mailboxes cannot be deleted through IMAP\r\n",
        ),
        (
            "RENAME Missing Other",
            "m1 NO [NONEXISTENT] Mailbox does not exist\r\n",
        ),
        (
            "RENAME Laravel Other",
            "m1 NO [CANNOT] mailboxes cannot be renamed through IMAP\r\n",
        ),
    ] {
        assert_eq!(client.status_of("m1", command).await, expected, "{command}");
    }
    server.stop().await;
}

/// APPEND is refused before the client uploads its literal: TRYCREATE for a missing mailbox,
/// CANNOT for an existing one, and the session continues.
#[tokio::test]
async fn append_is_refused_without_upload() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.connect().await;
    client.login().await;
    client.send(b"a1 APPEND Missing (\\Seen) {310}\r\n").await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "a1 NO [TRYCREATE] Mailbox does not exist\r\n"
    );
    client.send(b"a2 APPEND INBOX {310}\r\n").await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "a2 NO [CANNOT] Messages cannot be appended: Sandpost mailboxes show captured mail only\r\n"
    );
    client.send(b"a3 APPEND Trash {5+}\r\nHello\r\n").await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "a3 NO [CANNOT] Messages cannot be appended: Sandpost mailboxes show captured mail only\r\n"
    );
    assert_eq!(
        client.status_of("a4", "NOOP").await,
        "a4 OK NOOP completed\r\n"
    );
    assert!(server.backend.uids("INBOX").is_empty());
    server.stop().await;
}

/// A selected mailbox that disappears expunges every message at the next allowed point.
#[tokio::test]
async fn vanished_mailbox_expunges_everything() {
    let server = RunningServer::start(test_limits()).await;
    server.backend.deliver("Laravel", &plain_message("one"), 0);
    server.backend.deliver("Laravel", &plain_message("two"), 0);
    let mut client = server.selected("Laravel").await;
    server.backend.delete_mailbox("Laravel");
    assert_eq!(
        client.command("v1", "NOOP").await,
        [
            "* 2 EXPUNGE\r\n",
            "* 1 EXPUNGE\r\n",
            "v1 OK NOOP completed\r\n"
        ]
    );
    assert_eq!(
        client.command("v2", "UID FETCH 1:* FLAGS").await,
        ["v2 OK FETCH completed\r\n"]
    );
    server.stop().await;
}
