//! Message command conformance: FETCH, STORE, SEARCH, COPY, MOVE, EXPUNGE, and their UID forms
//! (RFC 3501 section 6.4, RFC 4315 UIDPLUS, RFC 6851 MOVE), plus EXPUNGE deferral rules
//! (RFC 3501 section 7.4.1).
mod support;

use sandpost_imap::{Account, AuthenticationOutcome, Credentials, Flags, ImapBackend, SystemFlag};
use support::{MULTIPART_MESSAGE, RunningServer, plain_message, test_limits};

/// FETCH returns sizes, dates, flags, envelope, and structure; BODY.PEEK leaves \Seen alone
/// while BODY[] sets it and reports the flag change.
#[tokio::test]
async fn fetch_items_and_implicit_seen() {
    let server = RunningServer::start(test_limits()).await;
    server
        .backend
        .deliver("INBOX", MULTIPART_MESSAGE, 1_704_794_400);
    let mut client = server.selected("INBOX").await;
    let responses = client
        .command("f1", "FETCH 1 (UID FLAGS INTERNALDATE RFC822.SIZE)")
        .await;
    assert_eq!(
        responses,
        [
            format!(
                "* 1 FETCH (UID 1 FLAGS (\\Recent) INTERNALDATE \" 9-Jan-2024 10:00:00 +0000\" RFC822.SIZE {})\r\n",
                MULTIPART_MESSAGE.len()
            ),
            "f1 OK FETCH completed\r\n".to_owned()
        ]
    );
    let envelope = client.command("f2", "FETCH 1 ENVELOPE").await;
    assert_eq!(
        envelope[0],
        "* 1 FETCH (ENVELOPE (\"Tue, 9 Jan 2024 10:00:00 +0000\" \"Password reset\" ((\"Laravel App\" NIL \"app\" \"laravel.test\")) ((\"Laravel App\" NIL \"app\" \"laravel.test\")) ((\"Laravel App\" NIL \"app\" \"laravel.test\")) ((\"Developer\" NIL \"developer\" \"example.test\")) NIL NIL NIL \"<reset-1@laravel.test>\"))\r\n"
    );
    let structure = client.command("f3", "FETCH 1 BODYSTRUCTURE").await;
    assert_eq!(
        structure[0],
        "* 1 FETCH (BODYSTRUCTURE ((\"text\" \"plain\" (\"charset\" \"utf-8\") NIL NIL \"7BIT\" 25 1 NIL NIL NIL NIL)(\"text\" \"html\" (\"charset\" \"utf-8\") NIL NIL \"7BIT\" 39 1 NIL NIL NIL NIL) \"alternative\" (\"boundary\" \"b1\") NIL NIL NIL))\r\n"
    );
    let peek = client
        .command(
            "f4",
            "FETCH 1 (BODY.PEEK[1] BODY.PEEK[HEADER.FIELDS (SUBJECT)])",
        )
        .await;
    assert_eq!(
        peek[0],
        "* 1 FETCH (BODY[1] {25}\r\nReset your password here. BODY[HEADER.FIELDS (SUBJECT)] {27}\r\nSubject: Password reset\r\n\r\n)\r\n"
    );
    assert!(server.backend.flags("INBOX", 1).unwrap().is_empty());
    let body = client.command("f5", "FETCH 1 BODY[2]<3.10>").await;
    assert_eq!(
        body[0],
        "* 1 FETCH (BODY[2]<3> {10}\r\nReset your FLAGS (\\Seen \\Recent))\r\n"
    );
    assert!(
        server
            .backend
            .flags("INBOX", 1)
            .unwrap()
            .contains(SystemFlag::Seen)
    );
    let full = client
        .command("f6", "FETCH 1 (RFC822.HEADER BODY[TEXT] BODY[3])")
        .await;
    assert!(full[0].contains("RFC822.HEADER {"), "{}", full[0]);
    assert!(full[0].contains("BODY[3] NIL"), "{}", full[0]);
    server.stop().await;
}

/// UID FETCH always includes UID, ignores unknown UIDs, and FETCH rejects numbers past the end.
#[tokio::test]
async fn uid_fetch_and_sequence_bounds() {
    let server = RunningServer::start(test_limits()).await;
    for subject in ["one", "two", "three"] {
        server.backend.deliver("INBOX", &plain_message(subject), 0);
    }
    let mut client = server.selected("INBOX").await;
    assert_eq!(
        client.command("u1", "UID FETCH 2:9 FLAGS").await,
        [
            "* 2 FETCH (UID 2 FLAGS (\\Recent))\r\n",
            "* 3 FETCH (UID 3 FLAGS (\\Recent))\r\n",
            "u1 OK FETCH completed\r\n"
        ]
    );
    assert_eq!(
        client.command("u2", "UID FETCH 100:* (FLAGS)").await,
        [
            "* 3 FETCH (UID 3 FLAGS (\\Recent))\r\n",
            "u2 OK FETCH completed\r\n"
        ]
    );
    assert_eq!(
        client.status_of("u3", "FETCH 4 FLAGS").await,
        "u3 BAD Invalid message sequence number\r\n"
    );
    let all = client.command("u4", "FETCH 1:* FAST").await;
    assert_eq!(all.len(), 4);
    server.stop().await;
}

/// STORE replaces, adds, and removes flags, announces new keywords, and honors .SILENT;
/// EXAMINE refuses STORE.
#[tokio::test]
async fn store_flags_and_keywords() {
    let server = RunningServer::start(test_limits()).await;
    server.backend.deliver("INBOX", &plain_message("one"), 0);
    server.backend.deliver("INBOX", &plain_message("two"), 0);
    let mut client = server.selected("INBOX").await;
    assert_eq!(
        client.command("s1", "STORE 1:2 +FLAGS (\\Flagged)").await,
        [
            "* 1 FETCH (FLAGS (\\Flagged \\Recent))\r\n",
            "* 2 FETCH (FLAGS (\\Flagged \\Recent))\r\n",
            "s1 OK STORE completed\r\n"
        ]
    );
    assert_eq!(
        client
            .command("s2", "UID STORE 2 FLAGS ($Junk \\Seen)")
            .await,
        [
            "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft $Junk)\r\n",
            "* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft $Junk \\*)] Flags permitted\r\n",
            "* 2 FETCH (UID 2 FLAGS (\\Seen \\Recent $Junk))\r\n",
            "s2 OK STORE completed\r\n"
        ]
    );
    assert_eq!(
        client
            .command("s3", "STORE 1 -FLAGS.SILENT (\\Flagged)")
            .await,
        ["s3 OK STORE completed\r\n"]
    );
    assert!(server.backend.flags("INBOX", 1).unwrap().is_empty());
    client.status_of("s4", "EXAMINE INBOX").await;
    assert_eq!(
        client.status_of("s5", "STORE 1 +FLAGS (\\Seen)").await,
        "s5 NO Mailbox is read-only\r\n"
    );
    server.stop().await;
}

/// SEARCH and UID SEARCH evaluate flags, dates, sizes, headers, and bodies; unsupported
/// charsets get BADCHARSET.
#[tokio::test]
async fn search_criteria() {
    let server = RunningServer::start(test_limits()).await;
    server
        .backend
        .deliver("INBOX", MULTIPART_MESSAGE, 1_704_794_400);
    server
        .backend
        .deliver("INBOX", &plain_message("Invoice paid"), 1_704_880_800);
    server
        .backend
        .deliver("INBOX", &plain_message("Welcome"), 1_704_967_200);
    let mut client = server.selected("INBOX").await;
    client.status_of("x", "STORE 2 +FLAGS (\\Seen)").await;
    for (command, expected) in [
        ("SEARCH ALL", "* SEARCH 1 2 3\r\n"),
        ("SEARCH UNSEEN", "* SEARCH 1 3\r\n"),
        ("SEARCH SUBJECT invoice", "* SEARCH 2\r\n"),
        ("SEARCH FROM \"laravel app\"", "* SEARCH 1\r\n"),
        ("SEARCH BODY \"RESET YOUR\"", "* SEARCH 1\r\n"),
        ("SEARCH TEXT welcome", "* SEARCH 3\r\n"),
        ("SEARCH HEADER Message-ID reset-1", "* SEARCH 1\r\n"),
        ("SEARCH SINCE 10-Jan-2024", "* SEARCH 2 3\r\n"),
        ("SEARCH BEFORE 10-Jan-2024", "* SEARCH 1\r\n"),
        ("SEARCH SENTON 9-Jan-2024", "* SEARCH 1\r\n"),
        ("SEARCH LARGER 200", "* SEARCH 1\r\n"),
        ("SEARCH OR SUBJECT welcome SEEN", "* SEARCH 2 3\r\n"),
        ("SEARCH NOT 1:2", "* SEARCH 3\r\n"),
        ("SEARCH NEW", "* SEARCH 1 3\r\n"),
        (
            "SEARCH CHARSET UTF-8 SUBJECT \"password\"",
            "* SEARCH 1\r\n",
        ),
        ("UID SEARCH UID 2:*", "* SEARCH 2 3\r\n"),
        ("SEARCH KEYWORD $Nothing", "* SEARCH\r\n"),
    ] {
        let responses = client.command("q", command).await;
        assert_eq!(responses[0], expected, "{command}");
        assert_eq!(responses[1], "q OK SEARCH completed\r\n");
    }
    assert_eq!(
        client.status_of("q2", "SEARCH CHARSET KOI8-R ALL").await,
        "q2 NO [BADCHARSET (US-ASCII UTF-8)] Unsupported charset\r\n"
    );
    server.stop().await;
}

/// COPY returns COPYUID; copies into a mailbox the backend refuses get CANNOT and missing ones
/// get TRYCREATE.
#[tokio::test]
async fn copy_reports_copyuid() {
    let server = RunningServer::start(test_limits()).await;
    server.backend.deliver("INBOX", &plain_message("one"), 0);
    server.backend.deliver("INBOX", &plain_message("two"), 0);
    let mut client = server.selected("INBOX").await;
    assert_eq!(
        client.command("c1", "COPY 1:2 Trash").await,
        ["c1 OK [COPYUID 1002 1:2 1:2] COPY completed\r\n"]
    );
    assert_eq!(
        client.command("c2", "UID COPY 2 Trash").await,
        ["c2 OK [COPYUID 1002 2 3] COPY completed\r\n"]
    );
    assert_eq!(
        client.status_of("c3", "COPY 1 Laravel").await,
        "c3 NO [CANNOT] Laravel is a dynamic folder\r\n"
    );
    assert_eq!(
        client.status_of("c4", "COPY 1 Missing").await,
        "c4 NO [TRYCREATE] Destination mailbox does not exist\r\n"
    );
    assert_eq!(
        client.status_of("c5", "UID COPY 99 Trash").await,
        "c5 OK COPY completed\r\n"
    );
    assert_eq!(server.backend.uids("Trash"), [1, 2, 3]);
    server.stop().await;
}

/// MOVE sends COPYUID in an untagged OK, then EXPUNGE responses, then the tagged OK.
#[tokio::test]
async fn move_expunges_from_source() {
    let server = RunningServer::start(test_limits()).await;
    for subject in ["one", "two", "three"] {
        server.backend.deliver("INBOX", &plain_message(subject), 0);
    }
    let mut client = server.selected("INBOX").await;
    assert_eq!(
        client.command("m1", "UID MOVE 1,3 Trash").await,
        [
            "* OK [COPYUID 1002 1,3 1:2] Moved\r\n",
            "* 3 EXPUNGE\r\n",
            "* 1 EXPUNGE\r\n",
            "m1 OK MOVE completed\r\n"
        ]
    );
    assert_eq!(server.backend.uids("INBOX"), [2]);
    client.status_of("m2", "EXAMINE INBOX").await;
    assert_eq!(
        client.status_of("m3", "MOVE 1 Trash").await,
        "m3 NO Mailbox is read-only\r\n"
    );
    server.stop().await;
}

/// EXPUNGE removes \Deleted messages; UID EXPUNGE limits removal to the given UIDs.
#[tokio::test]
async fn expunge_and_uid_expunge() {
    let server = RunningServer::start(test_limits()).await;
    for subject in ["one", "two", "three", "four"] {
        server.backend.deliver("INBOX", &plain_message(subject), 0);
    }
    let mut client = server.selected("INBOX").await;
    client
        .status_of("e1", "STORE 1:4 +FLAGS.SILENT (\\Deleted)")
        .await;
    assert_eq!(
        client.command("e2", "UID EXPUNGE 2:3").await,
        [
            "* 3 EXPUNGE\r\n",
            "* 2 EXPUNGE\r\n",
            "e2 OK EXPUNGE completed\r\n"
        ]
    );
    assert_eq!(
        client.command("e3", "EXPUNGE").await,
        [
            "* 2 EXPUNGE\r\n",
            "* 1 EXPUNGE\r\n",
            "e3 OK EXPUNGE completed\r\n"
        ]
    );
    assert!(server.backend.uids("INBOX").is_empty());
    server.stop().await;
}

/// Removals by others are announced by NOOP but deferred during FETCH, STORE, and SEARCH
/// (RFC 3501 section 7.4.1); UID FETCH may announce them.
#[tokio::test]
async fn foreign_expunges_respect_sequence_number_rules() {
    let server = RunningServer::start(test_limits()).await;
    for subject in ["one", "two", "three"] {
        server.backend.deliver("INBOX", &plain_message(subject), 0);
    }
    let mut client = server.selected("INBOX").await;
    server.backend.remove("INBOX", 2);
    let fetched = client.command("d1", "FETCH 1:3 (UID)").await;
    assert_eq!(
        fetched,
        [
            "* 1 FETCH (UID 1)\r\n",
            "* 3 FETCH (UID 3)\r\n",
            "d1 NO [EXPUNGEISSUED] Some messages were removed and could not be fetched\r\n"
        ]
    );
    let searched = client.command("d2", "SEARCH ALL").await;
    assert_eq!(searched[0], "* SEARCH 1 3\r\n");
    assert_eq!(
        client.command("d3", "NOOP").await,
        ["* 2 EXPUNGE\r\n", "d3 OK NOOP completed\r\n"]
    );
    server.backend.remove("INBOX", 3);
    assert_eq!(
        client.command("d4", "UID FETCH 1:* (FLAGS)").await,
        [
            "* 2 EXPUNGE\r\n",
            "* 1 FETCH (UID 1 FLAGS (\\Recent))\r\n",
            "d4 OK FETCH completed\r\n"
        ]
    );
    server.stop().await;
}

/// Flags changed and messages delivered by others are announced at the next command.
#[tokio::test]
async fn foreign_changes_are_announced() {
    let server = RunningServer::start(test_limits()).await;
    server.backend.deliver("INBOX", &plain_message("one"), 0);
    let mut client = server.selected("INBOX").await;
    let mut answered = Flags::new();
    answered.insert(SystemFlag::Answered);
    server.backend.set_flags("INBOX", 1, answered);
    server.backend.deliver("INBOX", &plain_message("two"), 0);
    assert_eq!(
        client.command("n1", "NOOP").await,
        [
            "* 1 FETCH (UID 1 FLAGS (\\Answered \\Recent))\r\n",
            "* 2 EXISTS\r\n",
            "* 2 RECENT\r\n",
            "n1 OK NOOP completed\r\n"
        ]
    );
    let second = server.selected("INBOX").await;
    drop(second);
    server.stop().await;
}

/// COPY and MOVE resolve received sequence numbers before announcing foreign removals.
#[tokio::test]
async fn copy_and_move_preserve_targets_across_foreign_expunges() {
    for remove_from_source in [false, true] {
        let server = RunningServer::start(test_limits()).await;
        for subject in ["one", "two", "three"] {
            server.backend.deliver("INBOX", &plain_message(subject), 0);
        }
        let mut client = server.selected("INBOX").await;
        server.backend.remove("INBOX", 1);
        if remove_from_source {
            assert_eq!(
                client.command("m", "MOVE 2 Trash").await,
                [
                    "* OK [COPYUID 1002 2 1] Moved\r\n",
                    "* 2 EXPUNGE\r\n",
                    "* 1 EXPUNGE\r\n",
                    "m OK MOVE completed\r\n",
                ]
            );
            assert_eq!(server.backend.uids("INBOX"), [3]);
        } else {
            assert_eq!(
                client.command("c", "COPY 2 Trash").await,
                ["c OK [COPYUID 1002 2 1] COPY completed\r\n"]
            );
            assert_eq!(server.backend.uids("INBOX"), [2, 3]);
            assert_eq!(
                client.command("n", "NOOP").await,
                ["* 1 EXPUNGE\r\n", "n OK NOOP completed\r\n"]
            );
        }
        assert_eq!(server.backend.uids("Trash"), [1]);
        server.stop().await;
    }
}

/// A removed COPY or MOVE target cannot be replaced by the message renumbered into its slot.
#[tokio::test]
async fn copy_and_move_refuse_removed_sequence_targets() {
    for command in ["COPY 2 Trash", "MOVE 2 Trash"] {
        let server = RunningServer::start(test_limits()).await;
        for subject in ["one", "two", "three"] {
            server.backend.deliver("INBOX", &plain_message(subject), 0);
        }
        let mut client = server.selected("INBOX").await;
        server.backend.remove("INBOX", 2);
        assert_eq!(
            client.command("c", command).await,
            ["c NO [EXPUNGEISSUED] Some messages were removed; nothing was copied\r\n"]
        );
        assert!(server.backend.uids("Trash").is_empty());
        assert_eq!(server.backend.uids("INBOX"), [1, 3]);
        server.stop().await;
    }
}

/// UID SEARCH retains sequence positions throughout bare and nested boolean criteria.
#[tokio::test]
async fn uid_search_sequence_keys_defer_foreign_expunges() {
    let server = RunningServer::start(test_limits()).await;
    for subject in ["one", "two", "three"] {
        server.backend.deliver("INBOX", &plain_message(subject), 0);
    }
    let mut client = server.selected("INBOX").await;
    server.backend.remove("INBOX", 1);
    for (command, expected) in [
        ("UID SEARCH 2", "* SEARCH 2\r\n"),
        ("UID SEARCH NOT 2", "* SEARCH 3\r\n"),
        ("UID SEARCH OR 1 2", "* SEARCH 2\r\n"),
        ("UID SEARCH (OR 1 3) NOT 2", "* SEARCH 3\r\n"),
        ("UID SEARCH 2 UID 2:3", "* SEARCH 2\r\n"),
        ("UID SEARCH UID 2:3", "* SEARCH 2 3\r\n"),
    ] {
        assert_eq!(
            client.command("s", command).await,
            [expected, "s OK SEARCH completed\r\n"],
            "{command}"
        );
    }
    assert_eq!(
        client.command("n", "NOOP").await,
        ["* 1 EXPUNGE\r\n", "n OK NOOP completed\r\n"]
    );
    server.stop().await;
}

/// UID star keys use the highest surviving UID while deferred sequence positions stay stable.
#[tokio::test]
async fn search_uid_star_ignores_pending_highest_uid_removal() {
    let server = RunningServer::start(test_limits()).await;
    for subject in ["one", "two", "three"] {
        server.backend.deliver("INBOX", &plain_message(subject), 0);
    }
    let mut client = server.selected("INBOX").await;
    server.backend.remove("INBOX", 3);
    for command in [
        "SEARCH UID *",
        "SEARCH UID 100:*",
        "UID SEARCH UID *",
        "UID SEARCH UID 100:*",
    ] {
        assert_eq!(
            client.command("s", command).await,
            ["* SEARCH 2\r\n", "s OK SEARCH completed\r\n"],
            "{command}"
        );
    }
    server.backend.remove("INBOX", 2);
    server.backend.remove("INBOX", 1);
    assert_eq!(
        client.command("s", "UID SEARCH UID 100:*").await,
        ["* SEARCH\r\n", "s OK SEARCH completed\r\n"]
    );
    assert_eq!(
        client.command("n", "NOOP").await,
        [
            "* 3 EXPUNGE\r\n",
            "* 2 EXPUNGE\r\n",
            "* 1 EXPUNGE\r\n",
            "n OK NOOP completed\r\n",
        ]
    );
    server.stop().await;
}

/// SENT keys require a usable Date header instead of substituting the capture date.
#[tokio::test]
async fn sent_search_keys_do_not_fall_back_to_internal_date() {
    let server = RunningServer::start(test_limits()).await;
    for date_header in [
        "",
        "Date: invalid\r\n",
        "Date: Tue, 9 Jan 2024 10:00:00 +0000\r\n",
    ] {
        let mut message = date_header.as_bytes().to_vec();
        message.extend(plain_message("captured in November"));
        server.backend.deliver("INBOX", &message, 1_700_000_000);
    }
    let mut client = server.selected("INBOX").await;
    for (command, expected) in [
        ("SEARCH SENTBEFORE 15-Nov-2023", "* SEARCH\r\n"),
        ("SEARCH SENTON 14-Nov-2023", "* SEARCH\r\n"),
        ("UID SEARCH SENTSINCE 1-Jan-2023", "* SEARCH 3\r\n"),
        ("SEARCH SENTON 9-Jan-2024", "* SEARCH 3\r\n"),
        ("SEARCH ON 14-Nov-2023", "* SEARCH 1 2 3\r\n"),
    ] {
        assert_eq!(
            client.command("s", command).await,
            [expected, "s OK SEARCH completed\r\n"],
            "{command}"
        );
    }
    server.stop().await;
}

/// NUL-bearing raw mail is refused before literal rendering or Seen mutation, and stays intact.
#[tokio::test]
async fn fetch_refuses_nul_content_without_changing_captured_bytes() {
    use sandpost_imap::AuthenticationMechanism;
    use support::backend::{PASSWORD, USERNAME};

    let server = RunningServer::start(test_limits()).await;
    let message = b"Subject: Binary capture\r\n\r\noriginal\0bytes\r\n";
    server.backend.deliver("INBOX", message, 0);
    let mut client = server.selected("INBOX").await;
    for command in [
        "FETCH 1 RFC822",
        "UID FETCH 1 BODY[]",
        "FETCH 1 RFC822.TEXT",
        "FETCH 1 BODY.PEEK[TEXT]",
        "FETCH 1 (FLAGS BODY[HEADER])",
        "FETCH 1 ENVELOPE",
    ] {
        // TestClient independently validates every wire response with imap-codec.
        assert_eq!(
            client.command("f", command).await,
            ["f NO [CANNOT] Message contains NUL octets unavailable through IMAP4rev1\r\n"],
            "{command}"
        );
        assert!(server.backend.flags("INBOX", 1).unwrap().is_empty());
    }
    assert_eq!(
        client.command("f", "FETCH 1 (FLAGS RFC822.SIZE)").await,
        [
            format!(
                "* 1 FETCH (FLAGS (\\Recent) RFC822.SIZE {})\r\n",
                message.len()
            ),
            "f OK FETCH completed\r\n".to_owned(),
        ]
    );
    let AuthenticationOutcome::Authenticated(account) = server
        .backend
        .authenticate(Credentials {
            username: USERNAME.to_owned(),
            password: PASSWORD.to_owned(),
            mechanism: AuthenticationMechanism::Login,
            encrypted: false,
            remote_address: None,
        })
        .await
        .unwrap()
    else {
        panic!("test account must authenticate");
    };
    let content = account
        .content(&"INBOX".to_owned(), 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&*content.octets, message);
    server.stop().await;
}
