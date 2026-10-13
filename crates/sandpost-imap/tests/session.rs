//! Session-level conformance over TCP: greeting, capabilities, state rules, authentication,
//! command framing and literals, limits, timeouts, and shutdown (RFC 3501 sections 3, 5, 6.1,
//! 6.2, 7.1; RFC 4959; RFC 7888).
mod support;

use sandpost_imap::Limits;
use std::time::Duration;
use support::{RunningServer, backend::PASSWORD, test_limits};

/// Encode bytes as standard padded base64.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            if index <= chunk.len() {
                encoded.push(ALPHABET[(value >> (18 - 6 * index) & 63) as usize] as char);
            } else {
                encoded.push('=');
            }
        }
    }
    encoded
}

/// The greeting is an untagged OK carrying the capability list (RFC 3501 sections 7.1.1, 7.2.1).
#[tokio::test]
async fn greeting_announces_capabilities() {
    let server = RunningServer::start(test_limits()).await;
    let stream = tokio::net::TcpStream::connect(server.address)
        .await
        .unwrap();
    let mut client = support::client::TestClient::new(stream);
    let greeting = client.greeting().await;
    assert!(
        greeting.starts_with("* OK [CAPABILITY IMAP4rev1 "),
        "{greeting}"
    );
    assert!(greeting.contains("AUTH=PLAIN"));
    server.stop().await;
}

/// CAPABILITY lists exactly the implemented extensions before and after login, and never
/// advertises IMAP4rev2, ENABLE, CONDSTORE, LIST-EXTENDED, or SPECIAL-USE.
#[tokio::test]
async fn capability_lists_only_implemented_extensions() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.connect().await;
    let responses = client.command("a1", "CAPABILITY").await;
    assert_eq!(
        responses,
        [
            "* CAPABILITY IMAP4rev1 AUTH=PLAIN SASL-IR LITERAL- ID IDLE NAMESPACE UIDPLUS MOVE UNSELECT CHILDREN\r\n",
            "a1 OK CAPABILITY completed\r\n"
        ]
    );
    let login = client
        .command("a2", &format!("LOGIN developer \"{PASSWORD}\""))
        .await;
    assert_eq!(
        login,
        [
            "a2 OK [CAPABILITY IMAP4rev1 LITERAL- ID IDLE NAMESPACE UIDPLUS MOVE UNSELECT CHILDREN] Logged in\r\n"
        ]
    );
    let after = client.command("a3", "capability").await;
    for absent in [
        "IMAP4rev2",
        "ENABLE",
        "CONDSTORE",
        "LIST-EXTENDED",
        "SPECIAL-USE",
        "LITERAL+",
        "STARTTLS",
        "AUTH=",
    ] {
        assert!(!after[0].contains(absent), "{absent}: {after:?}");
    }
    server.stop().await;
}

/// Commands outside their state receive BAD, and unknown or malformed commands are rejected.
#[tokio::test]
async fn state_rules_and_malformed_commands() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.connect().await;
    for (command, expected) in [
        ("SELECT INBOX", "t1 BAD Command requires authentication"),
        (
            "FETCH 1 FLAGS",
            "t1 BAD Command requires a selected mailbox",
        ),
        ("LIST \"\" *", "t1 BAD Command requires authentication"),
        ("FROBNICATE", "t1 BAD Invalid command: unknown command"),
        (
            "NOOP extra",
            "t1 BAD Invalid command: unexpected extra arguments",
        ),
    ] {
        let completion = client.status_of("t1", command).await;
        assert!(completion.starts_with(expected), "{command}: {completion}");
    }
    client.send(b"\"bad tag\r\n").await;
    let untagged = client.read_response().await.unwrap();
    assert!(untagged.starts_with("* BAD"), "{untagged}");
    client.login().await;
    let selected_only = client.status_of("t2", "EXPUNGE").await;
    assert!(selected_only.starts_with("t2 BAD"), "{selected_only}");
    let again = client.status_of("t3", "LOGIN developer x").await;
    assert!(again.starts_with("t3 BAD Already authenticated"), "{again}");
    server.stop().await;
}

/// NOOP, CHECK, ID, NAMESPACE, and LOGOUT produce their required responses.
#[tokio::test]
async fn any_state_commands_and_logout() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.connect().await;
    assert_eq!(
        client.command("n1", "NOOP").await,
        ["n1 OK NOOP completed\r\n"]
    );
    assert_eq!(
        client
            .command("n2", "ID (\"name\" \"test\" \"version\" NIL)")
            .await,
        ["* ID (\"name\" \"Sandpost\")\r\n", "n2 OK ID completed\r\n"]
    );
    client.login().await;
    assert_eq!(
        client.command("n3", "NAMESPACE").await,
        [
            "* NAMESPACE ((\"\" \"/\")) NIL NIL\r\n",
            "n3 OK NAMESPACE completed\r\n"
        ]
    );
    assert_eq!(
        client.command("n4", "LOGOUT").await,
        ["* BYE Logging out\r\n", "n4 OK LOGOUT completed\r\n"]
    );
    client.expect_closed().await;
    server.stop().await;
}

/// LOGIN accepts quoted strings and literals; wrong passwords fail with AUTHENTICATIONFAILED and
/// the third failure closes the session.
#[tokio::test]
async fn login_failures_are_bounded() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.connect().await;
    for attempt in 1..=2 {
        let completion = client
            .status_of(&format!("f{attempt}"), "LOGIN developer wrong")
            .await;
        assert_eq!(
            completion,
            format!("f{attempt} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n")
        );
    }
    let responses = client.command("f3", "LOGIN developer wrong").await;
    assert_eq!(
        responses[0],
        "f3 NO [AUTHENTICATIONFAILED] Invalid credentials\r\n"
    );
    let bye = client.read_response().await.unwrap();
    assert_eq!(bye, "* BYE Too many authentication failures\r\n");
    client.expect_closed().await;

    let mut client = server.connect().await;
    client.send(b"l1 LOGIN {9}\r\n").await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "+ Ready for literal data\r\n"
    );
    client
        .send(format!("developer {{{}}}\r\n", PASSWORD.len()).as_bytes())
        .await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "+ Ready for literal data\r\n"
    );
    client.send(format!("{PASSWORD}\r\n").as_bytes()).await;
    let completion = client.until_tagged("l1").await.pop().unwrap();
    assert!(completion.starts_with("l1 OK"), "{completion}");
    server.stop().await;
}

/// Throttled and temporarily unavailable authentication are distinguished from bad passwords.
#[tokio::test]
async fn throttled_and_unavailable_authentication() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.connect().await;
    assert_eq!(
        client.status_of("t1", "LOGIN throttled x").await,
        "t1 NO [UNAVAILABLE] Too many failed logins, try again later\r\n"
    );
    assert_eq!(
        client.status_of("t2", "LOGIN unavailable x").await,
        "t2 NO [UNAVAILABLE] Temporarily unavailable, try again later\r\n"
    );
    server.stop().await;
}

/// AUTHENTICATE PLAIN works with an initial response (RFC 4959), with a continuation, and can be
/// cancelled; other mechanisms and foreign authorization identities are refused.
#[tokio::test]
async fn authenticate_plain_variants() {
    let server = RunningServer::start(test_limits()).await;
    let credentials = base64(format!("\0developer\0{PASSWORD}").as_bytes());
    let mut client = server.connect().await;
    let completion = client
        .status_of("a1", &format!("AUTHENTICATE PLAIN {credentials}"))
        .await;
    assert!(completion.starts_with("a1 OK [CAPABILITY"), "{completion}");

    let mut client = server.connect().await;
    client.send(b"a2 AUTHENTICATE PLAIN\r\n").await;
    assert_eq!(client.read_response().await.unwrap(), "+ \r\n");
    client.send(format!("{credentials}\r\n").as_bytes()).await;
    assert!(client.read_response().await.unwrap().starts_with("a2 OK"));

    let mut client = server.connect().await;
    client.send(b"a3 AUTHENTICATE PLAIN\r\n").await;
    client.read_response().await.unwrap();
    client.send(b"*\r\n").await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "a3 BAD Authentication cancelled\r\n"
    );
    assert_eq!(
        client.status_of("a4", "AUTHENTICATE CRAM-MD5").await,
        "a4 NO Unsupported authentication mechanism\r\n"
    );
    let foreign = base64(format!("admin\0developer\0{PASSWORD}").as_bytes());
    assert_eq!(
        client
            .status_of("a5", &format!("AUTHENTICATE PLAIN {foreign}"))
            .await,
        "a5 BAD Invalid PLAIN response\r\n"
    );
    assert_eq!(
        client.status_of("a6", "AUTHENTICATE PLAIN !!!!").await,
        "a6 BAD Invalid command: invalid initial response\r\n"
    );
    server.stop().await;
}

/// SASL continuations reject MIME-style whitespace and misplaced base64 padding.
#[tokio::test]
async fn authenticate_rejects_malformed_base64_continuations() {
    let server = RunningServer::start(test_limits()).await;
    let credentials = base64(format!("\0developer\0{PASSWORD}").as_bytes());
    for malformed in [
        format!("={credentials}"),
        format!("{} {}", &credentials[..4], &credentials[4..]),
        format!("{credentials}="),
    ] {
        let mut client = server.connect().await;
        client.send(b"a1 AUTHENTICATE PLAIN\r\n").await;
        assert_eq!(client.read_response().await.unwrap(), "+ \r\n");
        client.send(format!("{malformed}\r\n").as_bytes()).await;
        assert_eq!(
            client.read_response().await.unwrap(),
            "a1 BAD Invalid PLAIN response\r\n"
        );
        client.login().await;
    }
    server.stop().await;
}

/// Commands split into single bytes are reassembled, and pipelined commands complete in order.
#[tokio::test]
async fn partial_reads_and_pipelining() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.connect().await;
    for byte in b"p1 NOOP\r\n" {
        client.send(&[*byte]).await;
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(
        client.read_response().await.unwrap(),
        "p1 OK NOOP completed\r\n"
    );
    client
        .send(b"p2 NOOP\r\np3 CAPABILITY\r\np4 NOOP\r\n")
        .await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "p2 OK NOOP completed\r\n"
    );
    assert!(
        client
            .read_response()
            .await
            .unwrap()
            .starts_with("* CAPABILITY")
    );
    assert_eq!(
        client.read_response().await.unwrap(),
        "p3 OK CAPABILITY completed\r\n"
    );
    assert_eq!(
        client.read_response().await.unwrap(),
        "p4 OK NOOP completed\r\n"
    );
    server.stop().await;
}

/// LITERAL- accepts small non-synchronizing literals without a continuation and refuses large
/// ones with TOOBIG after reading them (RFC 7888 section 5).
#[tokio::test]
async fn non_synchronizing_literals_follow_literal_minus() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.connect().await;
    client
        .send(
            format!(
                "n1 LOGIN {{9+}}\r\ndeveloper {{{}+}}\r\n{PASSWORD}\r\n",
                PASSWORD.len()
            )
            .as_bytes(),
        )
        .await;
    let completion = client.read_response().await.unwrap();
    assert!(completion.starts_with("n1 OK"), "{completion}");
    let large = "x".repeat(5000);
    client
        .send(format!("n2 SEARCH TEXT {{5000+}}\r\n{large}\r\n").as_bytes())
        .await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "n2 BAD [TOOBIG] Non-synchronizing literals are limited to 4096 octets\r\n"
    );
    assert_eq!(
        client.status_of("n3", "NOOP").await,
        "n3 OK NOOP completed\r\n"
    );
    server.stop().await;
}

/// Overlong lines and oversized synchronizing literals are refused without losing the session.
#[tokio::test]
async fn oversized_input_is_refused() {
    let limits = Limits {
        maximum_line_size: 128,
        maximum_command_size: 1024,
        ..test_limits()
    };
    let server = RunningServer::start(limits).await;
    let mut client = server.connect().await;
    client
        .send(format!("o1 NOOP {}\r\n", "y".repeat(400)).as_bytes())
        .await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "* BAD Command line too long\r\n"
    );
    client.send(b"o2 LOGIN {4096}\r\n").await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "o2 BAD [TOOBIG] Command too large\r\n"
    );
    assert_eq!(
        client.status_of("o3", "NOOP").await,
        "o3 OK NOOP completed\r\n"
    );
    server.stop().await;
}

/// The command-size bound also covers the final line after an accepted literal.
#[tokio::test]
async fn command_size_includes_lines_after_literals() {
    let limits = Limits {
        maximum_line_size: 128,
        maximum_command_size: 32,
        ..test_limits()
    };
    let server = RunningServer::start(limits).await;
    let mut client = server.connect().await;
    client
        .send(format!("c1 LOGIN {{9+}}\r\ndeveloper \"{PASSWORD}\"\r\n").as_bytes())
        .await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "c1 BAD [TOOBIG] Command too large\r\n"
    );
    assert_eq!(
        client.status_of("c2", "NOOP").await,
        "c2 OK NOOP completed\r\n"
    );
    server.stop().await;
}

/// Unauthenticated sessions are logged out after their shorter timeout.
#[tokio::test]
async fn unauthenticated_sessions_time_out() {
    let limits = Limits {
        unauthenticated_timeout: Duration::from_millis(150),
        ..test_limits()
    };
    let server = RunningServer::start(limits).await;
    let mut client = server.connect().await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "* BYE Autologout; idle for too long\r\n"
    );
    client.expect_closed().await;
    server.stop().await;
}

/// Connections over the limit receive BYE immediately.
#[tokio::test]
async fn connection_limit_refuses_with_bye() {
    let limits = Limits {
        maximum_connection_count: 1,
        ..test_limits()
    };
    let server = RunningServer::start(limits).await;
    let _first = server.connect().await;
    let stream = tokio::net::TcpStream::connect(server.address)
        .await
        .unwrap();
    let mut second = support::client::TestClient::new(stream);
    assert_eq!(
        second.read_response().await.unwrap(),
        "* BYE Too many connections, try again later\r\n"
    );
    second.expect_closed().await;
    server.stop().await;
}

/// Shutdown sends BYE to waiting sessions, closes them, and releases the port.
#[tokio::test]
async fn shutdown_says_bye_and_releases_the_port() {
    let server = RunningServer::start(test_limits()).await;
    let address = server.address;
    let mut client = server.connect().await;
    client.login().await;
    server.stop().await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "* BYE Server shutting down\r\n"
    );
    client.expect_closed().await;
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
}

/// A revoked account is told BYE at its next command.
#[tokio::test]
async fn revoked_accounts_are_disconnected() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.connect().await;
    client.login().await;
    server.backend.revoke();
    client.send(b"r1 NOOP\r\n").await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "* BYE This IMAP account is no longer available\r\n"
    );
    client.expect_closed().await;
    let mut again = server.connect().await;
    assert_eq!(
        again
            .status_of("r2", &format!("LOGIN developer \"{PASSWORD}\""))
            .await,
        "r2 NO [AUTHENTICATIONFAILED] Invalid credentials\r\n"
    );
    server.stop().await;
}

/// Rejecting APPEND before its literal still revalidates and terminates a revoked account.
#[tokio::test]
async fn revoked_accounts_cannot_bypass_validation_with_append() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.connect().await;
    client.login().await;
    server.backend.revoke();
    client.send(b"r1 APPEND INBOX {10}\r\n").await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "* BYE This IMAP account is no longer available\r\n"
    );
    client.expect_closed().await;
    server.stop().await;
}

/// Twenty consecutive invalid commands close the connection.
#[tokio::test]
async fn repeated_garbage_closes_the_connection() {
    let server = RunningServer::start(test_limits()).await;
    let mut client = server.connect().await;
    for _ in 0..20 {
        client.send(b"x GARBAGE\r\n").await;
    }
    for _ in 0..20 {
        assert!(client.read_response().await.unwrap().starts_with("x BAD"));
    }
    assert_eq!(
        client.read_response().await.unwrap(),
        "* BYE Too many invalid commands\r\n"
    );
    client.expect_closed().await;
    server.stop().await;
}
