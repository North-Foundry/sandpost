//! SMTP AUTH: advertised mechanisms, PLAIN and LOGIN exchanges, policies, and failures.
use super::support::*;
use crate::{
    AuthenticationError, AuthenticationOutcome, AuthenticationPolicy, Credentials, DeliveryError,
    Limits, SessionHandler,
};
use sandpost_core::Message;
use std::{
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Record every credential that reaches the application and accept it for protocol tests.
struct RecordingAuthenticationHandler(Arc<Mutex<Vec<Credentials>>>);

impl SessionHandler for RecordingAuthenticationHandler {
    type Principal = String;

    /// Record credentials passed through strict protocol validation and accept their username.
    fn authenticate(
        &self,
        credentials: Credentials,
    ) -> impl Future<Output = Result<AuthenticationOutcome<String>, AuthenticationError>> + Send
    {
        let principal = credentials.username.clone();
        self.0.lock().unwrap().push(credentials);
        std::future::ready(Ok(AuthenticationOutcome::Authenticated(principal)))
    }

    /// Accept mail because these tests exercise authentication only.
    fn deliver(
        &self,
        _message: Message,
        _principal: Option<String>,
    ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
        std::future::ready(Ok(()))
    }
}

/// EHLO lists the supported extensions and only the AUTH mechanisms that hashes can verify.
#[tokio::test]
async fn ehlo_advertises_extensions_and_plain_and_login_authentication() {
    let mut reader = authenticating_test_client(
        AuthenticationPolicy::Optional,
        record_into(RecordedDeliveries::default()),
    )
    .await;
    read_mail_protocol_response_line(&mut reader).await;
    reader
        .get_mut()
        .write_all(b"EHLO client\r\n")
        .await
        .unwrap();
    assert_eq!(
        read_mail_protocol_reply_lines(&mut reader).await,
        [
            "250-sandpost.localhost",
            "250-PIPELINING",
            "250-SIZE 10485760",
            "250-8BITMIME",
            "250-ENHANCEDSTATUSCODES",
            "250 AUTH PLAIN LOGIN",
        ],
        "a plaintext server offers no STARTTLS"
    );
}

/// PLAIN with an initial response authenticates and binds the principal to delivered mail.
#[tokio::test]
async fn plain_authentication_binds_the_principal_to_delivered_mail() {
    let deliveries = RecordedDeliveries::default();
    let mut reader = authenticating_test_client(
        AuthenticationPolicy::Required,
        record_into(deliveries.clone()),
    )
    .await;
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    let response = send_mail_protocol_command(
        &mut reader,
        &format!(
            "AUTH PLAIN {}",
            plain_response("", TEST_USERNAME, TEST_PASSWORD)
        ),
    )
    .await;
    assert!(response.starts_with("235"), "{response}");
    assert!(
        submit_test_message(&mut reader, "authenticated")
            .await
            .starts_with("250")
    );
    // RSET and a new EHLO keep the authenticated identity for later transactions.
    send_mail_protocol_command(&mut reader, "RSET").await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    assert!(
        submit_test_message(&mut reader, "second")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "AUTH PLAIN =")
            .await
            .starts_with("503"),
        "a session authenticates at most once"
    );
    assert_eq!(
        *deliveries.lock().unwrap(),
        vec![
            ("authenticated".to_owned(), Some(TEST_USERNAME.to_owned())),
            ("second".to_owned(), Some(TEST_USERNAME.to_owned())),
        ]
    );
}

/// PLAIN without an initial response and LOGIN both use base64 continuation challenges.
#[tokio::test]
async fn challenge_based_plain_and_login_authenticate() {
    let mut reader = authenticating_test_client(
        AuthenticationPolicy::Required,
        record_into(RecordedDeliveries::default()),
    )
    .await;
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    assert_eq!(
        send_mail_protocol_command(&mut reader, "AUTH PLAIN").await,
        "334 \r\n"
    );
    let response = send_mail_protocol_command(
        &mut reader,
        &plain_response(TEST_USERNAME, TEST_USERNAME, TEST_PASSWORD),
    )
    .await;
    assert!(response.starts_with("235"), "{response}");

    let mut login_reader = authenticating_test_client(
        AuthenticationPolicy::Required,
        record_into(RecordedDeliveries::default()),
    )
    .await;
    read_mail_protocol_response_line(&mut login_reader).await;
    send_mail_protocol_command(&mut login_reader, "EHLO client").await;
    assert_eq!(
        send_mail_protocol_command(&mut login_reader, "AUTH LOGIN").await,
        "334 VXNlcm5hbWU6\r\n"
    );
    assert_eq!(
        send_mail_protocol_command(&mut login_reader, &base64_encode(TEST_USERNAME.as_bytes()))
            .await,
        "334 UGFzc3dvcmQ6\r\n"
    );
    let response =
        send_mail_protocol_command(&mut login_reader, &base64_encode(TEST_PASSWORD.as_bytes()))
            .await;
    assert!(response.starts_with("235"), "{response}");
    assert!(
        submit_test_message(&mut login_reader, "login")
            .await
            .starts_with("250")
    );
}

/// A required policy refuses unauthenticated mail; invalid credentials never authenticate.
#[tokio::test]
async fn required_policy_refuses_unauthenticated_and_invalid_sessions() {
    let deliveries = RecordedDeliveries::default();
    let mut reader = authenticating_test_client(
        AuthenticationPolicy::Required,
        record_into(deliveries.clone()),
    )
    .await;
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<app@example.test>")
            .await
            .starts_with("530")
    );
    let response = send_mail_protocol_command(
        &mut reader,
        &format!(
            "AUTH PLAIN {}",
            plain_response("", TEST_USERNAME, "wrong password")
        ),
    )
    .await;
    assert!(response.starts_with("535"), "{response}");
    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<app@example.test>")
            .await
            .starts_with("530"),
        "a failed AUTH leaves the session unauthenticated"
    );
    let response = send_mail_protocol_command(
        &mut reader,
        &format!(
            "AUTH PLAIN {}",
            plain_response("", "unknown", TEST_PASSWORD)
        ),
    )
    .await;
    assert!(response.starts_with("535"), "{response}");
    let response = send_mail_protocol_command(
        &mut reader,
        &format!(
            "AUTH PLAIN {}",
            plain_response("", TEST_USERNAME, "another wrong password")
        ),
    )
    .await;
    assert!(response.starts_with("421"), "{response}");
    assert_eq!(
        read_mail_protocol_response_line(&mut reader).await,
        "",
        "the session closes after repeated failures"
    );
    assert!(deliveries.lock().unwrap().is_empty());
}

/// An optional policy keeps unauthenticated local delivery and tolerates the AUTH= parameter.
#[tokio::test]
async fn optional_policy_delivers_without_a_principal() {
    let deliveries = RecordedDeliveries::default();
    let mut reader = authenticating_test_client(
        AuthenticationPolicy::Optional,
        record_into(deliveries.clone()),
    )
    .await;
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    assert!(
        submit_test_message(&mut reader, "anonymous")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<app@example.test> AUTH=<>")
            .await
            .starts_with("250")
    );
    assert_eq!(
        *deliveries.lock().unwrap(),
        vec![("anonymous".to_owned(), None)]
    );
}

/// Malformed, cancelled, out-of-sequence, and foreign-identity AUTH attempts are refused.
#[tokio::test]
async fn malformed_and_out_of_sequence_authentication_is_refused() {
    let mut reader = authenticating_test_client(
        AuthenticationPolicy::Optional,
        record_into(RecordedDeliveries::default()),
    )
    .await;
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "HELO client").await;
    assert!(
        send_mail_protocol_command(&mut reader, "AUTH PLAIN =")
            .await
            .starts_with("503"),
        "AUTH requires EHLO"
    );
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    assert!(
        send_mail_protocol_command(&mut reader, "AUTH CRAM-MD5")
            .await
            .starts_with("504")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "AUTH")
            .await
            .starts_with("501")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "AUTH PLAIN not-base64!")
            .await
            .starts_with("501")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "AUTH LOGIN")
            .await
            .starts_with("334")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "*")
            .await
            .starts_with("501")
    );
    let response = send_mail_protocol_command(
        &mut reader,
        &format!(
            "AUTH PLAIN {}",
            plain_response("someone-else", TEST_USERNAME, TEST_PASSWORD)
        ),
    )
    .await;
    assert!(response.starts_with("535"), "{response}");
    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<app@example.test>")
            .await
            .starts_with("250")
    );
    assert!(
        send_mail_protocol_command(
            &mut reader,
            &format!(
                "AUTH PLAIN {}",
                plain_response("", TEST_USERNAME, TEST_PASSWORD)
            )
        )
        .await
        .starts_with("503"),
        "AUTH is refused inside a mail transaction"
    );
}

/// A verification failure is a temporary 454 that neither authenticates nor counts as invalid.
#[tokio::test]
async fn unavailable_authentication_is_reported_temporarily() {
    struct UnavailableHandler;
    impl SessionHandler for UnavailableHandler {
        type Principal = ();
        /// Fail every verification as if storage were down.
        fn authenticate(
            &self,
            _credentials: Credentials,
        ) -> impl Future<Output = Result<AuthenticationOutcome<()>, AuthenticationError>> + Send
        {
            std::future::ready(Err(AuthenticationError::Unavailable(
                "storage offline".into(),
            )))
        }
        /// Accept every message.
        fn deliver(
            &self,
            _message: Message,
            _principal: Option<()>,
        ) -> impl Future<Output = Result<(), DeliveryError>> + Send {
            std::future::ready(Ok(()))
        }
    }
    let mut reader = start_session(
        UnavailableHandler,
        AuthenticationPolicy::Required,
        Limits::default(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    let credentials = plain_response("", TEST_USERNAME, TEST_PASSWORD);
    // More attempts than the failure limit: temporary failures must not close the session.
    for _ in 0..4 {
        assert!(
            send_mail_protocol_command(&mut reader, &format!("AUTH PLAIN {credentials}"))
                .await
                .starts_with("454 4.7.0")
        );
    }
    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<app@example.test>")
            .await
            .starts_with("530"),
        "the session stays unauthenticated"
    );
}

/// Per-credential refusals answer 538 and 534, do not count as failures, and the handler learns
/// which mechanism was used and that the session was not encrypted.
#[tokio::test]
async fn per_credential_rules_answer_538_and_534() {
    use sandpost_core::SmtpAuthenticationMechanism;
    let received = ReceivedAuthentications::default();
    let mut reader = start_session(
        RulesHandler(received.clone()),
        AuthenticationPolicy::Optional,
        Limits {
            maximum_authentication_failures: 1,
            ..Limits::default()
        },
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    assert!(
        send_mail_protocol_command(
            &mut reader,
            &format!("AUTH PLAIN {}", plain_response("", "tls-only", "secret"))
        )
        .await
        .starts_with("538 5.7.11")
    );
    assert!(
        send_mail_protocol_command(
            &mut reader,
            &format!("AUTH PLAIN {}", plain_response("", "login-only", "secret"))
        )
        .await
        .starts_with("534 5.7.9"),
        "refusals do not exhaust the failure limit of one"
    );
    assert!(
        send_mail_protocol_command(&mut reader, "AUTH LOGIN")
            .await
            .starts_with("334")
    );
    assert!(
        send_mail_protocol_command(&mut reader, &base64_encode(b"login-only"))
            .await
            .starts_with("334")
    );
    assert!(
        send_mail_protocol_command(&mut reader, &base64_encode(b"secret"))
            .await
            .starts_with("235")
    );
    assert_eq!(
        received.lock().unwrap().as_slice(),
        [
            (SmtpAuthenticationMechanism::Plain, false),
            (SmtpAuthenticationMechanism::Plain, false),
            (SmtpAuthenticationMechanism::Login, false),
        ]
    );
}

/// Invalid encodings and malformed credential fields never reach the application.
#[tokio::test]
async fn malformed_base64_and_empty_plain_fields_never_reach_authenticator() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let mut reader = start_session(
        RecordingAuthenticationHandler(received.clone()),
        AuthenticationPolicy::Required,
        Limits::default(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    // The last two fixtures encode otherwise valid PLAIN fields with nonzero unused bits.
    for response in ["AA=A", "AA==AAAA", "A===", "AHUAcA", "AHUAcB==", "AHUAcHB="] {
        assert!(
            send_mail_protocol_command(&mut reader, &format!("AUTH PLAIN {response}"))
                .await
                .starts_with("501"),
            "{response}"
        );
    }
    for response in [
        base64_encode(b"\0\0password"),
        base64_encode(b"\0username\0"),
        base64_encode(b"\0username\0password\0extra"),
        base64_encode(b"\0\xff\0password"),
        base64_encode(b"\0username\0\xff"),
        base64_encode(b"\xff\0username\0password"),
        plain_response(&"a".repeat(256), "u", "p"),
        plain_response("", &"u".repeat(256), "p"),
        plain_response("", "u", &"p".repeat(256)),
    ] {
        assert!(
            send_mail_protocol_command(&mut reader, &format!("AUTH PLAIN {response}"))
                .await
                .starts_with("501"),
            "malformed PLAIN credential field: {response}"
        );
    }
    for (username, password) in [
        (Vec::new(), b"password".to_vec()),
        (b"username".to_vec(), Vec::new()),
        (b"user\0name".to_vec(), b"password".to_vec()),
        (b"username".to_vec(), b"pass\0word".to_vec()),
    ] {
        assert!(
            send_mail_protocol_command(&mut reader, "AUTH LOGIN")
                .await
                .starts_with("334")
        );
        assert!(
            send_mail_protocol_command(&mut reader, &base64_encode(&username))
                .await
                .starts_with("334")
        );
        assert!(
            send_mail_protocol_command(&mut reader, &base64_encode(&password))
                .await
                .starts_with("501")
        );
    }
    assert!(received.lock().unwrap().is_empty());
}

/// Three maximum-size PLAIN fields fit the allowed 1024-byte continuation response.
#[tokio::test]
async fn plain_authentication_accepts_three_255_byte_fields() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let mut reader = start_session(
        RecordingAuthenticationHandler(received.clone()),
        AuthenticationPolicy::Required,
        Limits::default(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    let identity = "x".repeat(255);
    let password = "p".repeat(255);
    let encoded = plain_response(&identity, &identity, &password);
    assert_eq!(encoded.len(), 1024);
    assert_eq!(
        send_mail_protocol_command(&mut reader, "AUTH PLAIN").await,
        "334 \r\n"
    );
    assert!(
        send_mail_protocol_command(&mut reader, &encoded)
            .await
            .starts_with("235")
    );
    let credentials = received.lock().unwrap();
    assert_eq!(credentials.len(), 1);
    assert_eq!(credentials[0].username, identity);
    assert_eq!(credentials[0].password, password);
}

/// An oversized initial AUTH command is refused, while the same credentials work as a continuation.
#[tokio::test]
async fn oversized_initial_authentication_response_allows_a_subsequent_challenge_exchange() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let mut reader = start_session(
        RecordingAuthenticationHandler(received.clone()),
        AuthenticationPolicy::Required,
        Limits::default(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    let username = "u".repeat(255);
    let password = "p".repeat(255);
    let encoded = plain_response("", &username, &password);
    let command = format!("AUTH PLAIN {encoded}");
    assert!(command.len() + 2 > 512);
    assert_eq!(
        send_mail_protocol_command(&mut reader, &command).await,
        "500 5.5.2 command line too long\r\n"
    );
    assert!(received.lock().unwrap().is_empty());
    assert_eq!(
        send_mail_protocol_command(&mut reader, "AUTH PLAIN").await,
        "334 \r\n"
    );
    assert!(
        send_mail_protocol_command(&mut reader, &encoded)
            .await
            .starts_with("235")
    );
    let credentials = received.lock().unwrap();
    assert_eq!(credentials.len(), 1);
    assert_eq!(credentials[0].username, username);
    assert_eq!(credentials[0].password, password);
}

/// An overlong continuation receives a syntax reply and closes without processing its suffix.
#[tokio::test]
async fn overlong_authentication_continuation_is_replied_to_then_closed() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let mut reader = start_session(
        RecordingAuthenticationHandler(received.clone()),
        AuthenticationPolicy::Required,
        Limits::default(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    assert!(
        send_mail_protocol_command(&mut reader, "AUTH PLAIN")
            .await
            .starts_with("334")
    );
    reader
        .get_mut()
        .write_all(format!("{}\r\nNOOP\r\n", "A".repeat(13_000)).as_bytes())
        .await
        .unwrap();
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("500 5.5.6")
    );
    let mut trailing = String::new();
    reader.read_to_string(&mut trailing).await.unwrap();
    assert!(trailing.is_empty(), "the pipelined suffix must not execute");
    assert!(received.lock().unwrap().is_empty());
}

/// The RFC 4954 `=` initial response is an empty response, not an omitted response or credentials.
#[tokio::test]
async fn empty_plain_initial_response_is_refused_without_calling_authenticator() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let mut reader = start_session(
        RecordingAuthenticationHandler(received.clone()),
        AuthenticationPolicy::Required,
        Limits::default(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    assert!(
        send_mail_protocol_command(&mut reader, "AUTH PLAIN =")
            .await
            .starts_with("501"),
        "the empty PLAIN message has no required authcid or password"
    );
    assert!(received.lock().unwrap().is_empty());
    assert!(
        send_mail_protocol_command(
            &mut reader,
            &format!(
                "AUTH PLAIN {}",
                plain_response("", TEST_USERNAME, TEST_PASSWORD)
            )
        )
        .await
        .starts_with("235"),
        "a malformed exchange leaves the session available for AUTH retry"
    );
    assert_eq!(received.lock().unwrap().len(), 1);
}

/// PLAIN fields are measured in UTF-8 octets, including the 255-octet maximum.
#[tokio::test]
async fn plain_authentication_enforces_utf8_octet_limits() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let mut reader = start_session(
        RecordingAuthenticationHandler(received.clone()),
        AuthenticationPolicy::Required,
        Limits::default(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;

    let over_limit = "€".repeat(86);
    assert_eq!(over_limit.len(), 258);
    assert!(
        send_mail_protocol_command(
            &mut reader,
            &format!("AUTH PLAIN {}", plain_response(&over_limit, "u", "p"))
        )
        .await
        .starts_with("501"),
        "a UTF-8 authzid above 255 octets is rejected"
    );

    let maximum = "€".repeat(85);
    assert_eq!(maximum.len(), 255);
    assert_eq!(
        send_mail_protocol_command(&mut reader, "AUTH PLAIN").await,
        "334 \r\n"
    );
    assert!(
        send_mail_protocol_command(&mut reader, &plain_response(&maximum, &maximum, &maximum))
            .await
            .starts_with("235"),
        "each UTF-8 field at exactly 255 octets is accepted"
    );
    assert_eq!(received.lock().unwrap().len(), 1);
}

/// SMTP's PLAIN profile permits a matching authzid and refuses an attempt to act as another user.
#[tokio::test]
async fn plain_authentication_accepts_matching_and_refuses_foreign_authorization_identity() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let mut matching_reader = start_session(
        RecordingAuthenticationHandler(received.clone()),
        AuthenticationPolicy::Required,
        Limits::default(),
    );
    read_mail_protocol_response_line(&mut matching_reader).await;
    send_mail_protocol_command(&mut matching_reader, "EHLO client").await;
    assert!(
        send_mail_protocol_command(
            &mut matching_reader,
            &format!(
                "AUTH PLAIN {}",
                plain_response(TEST_USERNAME, TEST_USERNAME, TEST_PASSWORD)
            )
        )
        .await
        .starts_with("235")
    );
    assert_eq!(received.lock().unwrap().len(), 1);

    let mut foreign_reader = start_session(
        RecordingAuthenticationHandler(received.clone()),
        AuthenticationPolicy::Required,
        Limits::default(),
    );
    read_mail_protocol_response_line(&mut foreign_reader).await;
    send_mail_protocol_command(&mut foreign_reader, "EHLO client").await;
    assert!(
        send_mail_protocol_command(
            &mut foreign_reader,
            &format!(
                "AUTH PLAIN {}",
                plain_response("someone-else", TEST_USERNAME, TEST_PASSWORD)
            )
        )
        .await
        .starts_with("535")
    );
    assert_eq!(
        received.lock().unwrap().len(),
        1,
        "foreign authzid is refused before credentials reach the handler"
    );
}

/// A cancelled challenge ends only that AUTH exchange, so the client can retry on the session.
#[tokio::test]
async fn cancelled_plain_and_login_exchanges_allow_authentication_retry() {
    for mechanism in ["PLAIN", "LOGIN"] {
        let received = Arc::new(Mutex::new(Vec::new()));
        let mut reader = start_session(
            RecordingAuthenticationHandler(received.clone()),
            AuthenticationPolicy::Required,
            Limits::default(),
        );
        read_mail_protocol_response_line(&mut reader).await;
        send_mail_protocol_command(&mut reader, "EHLO client").await;

        assert!(
            send_mail_protocol_command(&mut reader, &format!("AUTH {mechanism}"))
                .await
                .starts_with("334")
        );
        if mechanism == "LOGIN" {
            assert!(
                send_mail_protocol_command(&mut reader, &base64_encode(TEST_USERNAME.as_bytes()))
                    .await
                    .starts_with("334")
            );
        }
        assert!(
            send_mail_protocol_command(&mut reader, "*")
                .await
                .starts_with("501 5.7.0")
        );
        assert!(received.lock().unwrap().is_empty());
        assert!(
            send_mail_protocol_command(
                &mut reader,
                &format!(
                    "AUTH PLAIN {}",
                    plain_response("", TEST_USERNAME, TEST_PASSWORD)
                )
            )
            .await
            .starts_with("235"),
            "retry after cancelling {mechanism}"
        );
        assert_eq!(received.lock().unwrap().len(), 1);
    }
}

/// RFC 4954 permits a one-round-trip PLAIN AUTH with an initial response before another pipelined command.
#[tokio::test]
async fn plain_initial_response_preserves_pipelined_command_boundary() {
    let mut reader = start_session(
        RecordingAuthenticationHandler(Arc::new(Mutex::new(Vec::new()))),
        AuthenticationPolicy::Required,
        Limits::default(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    let auth = format!(
        "AUTH PLAIN {}\r\nNOOP\r\n",
        plain_response("", TEST_USERNAME, TEST_PASSWORD)
    );
    reader.get_mut().write_all(auth.as_bytes()).await.unwrap();
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("235")
    );
    assert_eq!(
        read_mail_protocol_response_line(&mut reader).await,
        "250 2.0.0 OK\r\n",
        "the pipelined NOOP is parsed after the AUTH response"
    );
}

/// RFC 4954 recommends marking a successfully authenticated delivery as ESMTPA in its trace.
#[tokio::test]
async fn authenticated_delivery_uses_esmtpa_trace_protocol() {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let captured = messages.clone();
    let handler = ClosureHandler(move |message: Message, principal: Option<String>| {
        assert_eq!(principal.as_deref(), Some(TEST_USERNAME));
        captured.lock().unwrap().push(message.raw_message);
        std::future::ready(Ok(()))
    });
    let mut reader = start_session(handler, AuthenticationPolicy::Required, Limits::default());
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    assert!(
        send_mail_protocol_command(
            &mut reader,
            &format!(
                "AUTH PLAIN {}",
                plain_response("", TEST_USERNAME, TEST_PASSWORD)
            )
        )
        .await
        .starts_with("235")
    );
    assert!(
        submit_test_message(&mut reader, "authenticated trace")
            .await
            .starts_with("250")
    );
    let messages = messages.lock().unwrap();
    let raw_message = std::str::from_utf8(&messages[0]).unwrap();
    assert!(raw_message.contains("with ESMTPA;"), "{raw_message}");
}
