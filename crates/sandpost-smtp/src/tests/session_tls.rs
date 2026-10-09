//! STARTTLS, implicit TLS, and the AUTH and MAIL requirements tied to encryption.
use super::support::*;
use crate::{AuthenticationPolicy, Limits, TransportSecurity};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Start a STARTTLS session that accepts the fixed test credential and records deliveries.
fn start_tls_session(
    certificate: &TestCertificate,
    authentication: AuthenticationPolicy,
    required: bool,
    deliveries: RecordedDeliveries,
) -> tokio::io::BufReader<tokio::io::DuplexStream> {
    start_secured_session(
        ClosureHandler(record_into(deliveries)),
        authentication,
        TransportSecurity::StartTls {
            configuration: certificate.configuration.clone(),
            required,
        },
        Limits::default(),
    )
}

/// STARTTLS is advertised in clear text, upgrades the connection, and is not offered again.
#[tokio::test]
async fn start_tls_upgrades_the_session_and_mail_flows_encrypted() {
    let certificate = test_certificate();
    let deliveries = RecordedDeliveries::default();
    let mut reader = start_tls_session(
        &certificate,
        AuthenticationPolicy::Optional,
        false,
        deliveries.clone(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    reader
        .get_mut()
        .write_all(b"EHLO client\r\n")
        .await
        .unwrap();
    let capabilities = read_mail_protocol_reply_lines(&mut reader).await;
    assert!(
        capabilities.contains(&"250-STARTTLS".to_owned()),
        "{capabilities:?}"
    );
    assert!(
        capabilities.contains(&"250 AUTH PLAIN LOGIN".to_owned()),
        "optional TLS keeps AUTH"
    );
    assert!(
        send_mail_protocol_command(&mut reader, "STARTTLS")
            .await
            .starts_with("220 2.0.0")
    );
    assert!(reader.buffer().is_empty());
    let mut encrypted = connect_with_tls(&certificate.connector, reader.into_inner()).await;
    encrypted
        .get_mut()
        .write_all(b"EHLO client\r\n")
        .await
        .unwrap();
    let capabilities = read_mail_protocol_reply_lines(&mut encrypted).await;
    assert!(
        !capabilities.iter().any(|line| line.ends_with("STARTTLS")),
        "{capabilities:?}"
    );
    assert!(capabilities.contains(&"250 AUTH PLAIN LOGIN".to_owned()));
    assert!(
        send_mail_protocol_command(&mut encrypted, "STARTTLS")
            .await
            .starts_with("503 5.5.1")
    );
    let credentials = plain_response("", TEST_USERNAME, TEST_PASSWORD);
    assert!(
        send_mail_protocol_command(&mut encrypted, &format!("AUTH PLAIN {credentials}"))
            .await
            .starts_with("235")
    );
    assert!(
        submit_test_message(&mut encrypted, "over tls")
            .await
            .starts_with("250 2.0.0")
    );
    assert_eq!(
        deliveries.lock().unwrap().as_slice(),
        [("over tls".to_owned(), Some(TEST_USERNAME.to_owned()))]
    );
}

/// TLS 1.2 and TLS 1.3 each protect SMTP after either STARTTLS or an implicit handshake.
#[tokio::test]
async fn tls_12_and_tls_13_complete_handshake_and_smtp() {
    use tokio_rustls::rustls::{
        ClientConfig, RootCertStore,
        crypto::ring,
        pki_types::CertificateDer,
        pki_types::pem::PemObject,
        version::{TLS12, TLS13},
    };

    let certificate = test_certificate();
    let trusted_certificate =
        CertificateDer::pem_slice_iter(certificate.certificate_pem.as_bytes())
            .next()
            .unwrap()
            .unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(trusted_certificate).unwrap();

    for version in [&TLS12, &TLS13] {
        let client_configuration =
            ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
                .with_protocol_versions(&[version])
                .unwrap()
                .with_root_certificates(roots.clone())
                .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(client_configuration));

        for implicit in [false, true] {
            let deliveries = RecordedDeliveries::default();
            let transport_security = if implicit {
                TransportSecurity::Implicit {
                    configuration: certificate.configuration.clone(),
                }
            } else {
                TransportSecurity::StartTls {
                    configuration: certificate.configuration.clone(),
                    required: false,
                }
            };
            let mut reader = start_secured_session(
                ClosureHandler(record_into(deliveries.clone())),
                AuthenticationPolicy::Optional,
                transport_security,
                Limits::default(),
            );
            if implicit {
                let mut encrypted = connect_with_tls(&connector, reader.into_inner()).await;
                assert_eq!(
                    encrypted.get_ref().get_ref().1.protocol_version(),
                    Some(version.version)
                );
                read_mail_protocol_response_line(&mut encrypted).await;
                send_mail_protocol_command(&mut encrypted, "EHLO client").await;
                assert!(
                    submit_test_message(&mut encrypted, "versioned tls")
                        .await
                        .starts_with("250")
                );
            } else {
                read_mail_protocol_response_line(&mut reader).await;
                send_mail_protocol_command(&mut reader, "EHLO client").await;
                assert!(
                    send_mail_protocol_command(&mut reader, "STARTTLS")
                        .await
                        .starts_with("220")
                );
                let mut encrypted = connect_with_tls(&connector, reader.into_inner()).await;
                assert_eq!(
                    encrypted.get_ref().get_ref().1.protocol_version(),
                    Some(version.version)
                );
                send_mail_protocol_command(&mut encrypted, "EHLO client").await;
                assert!(
                    submit_test_message(&mut encrypted, "versioned tls")
                        .await
                        .starts_with("250")
                );
            }
            assert_eq!(deliveries.lock().unwrap().len(), 1);
        }
    }
}

/// RFC 8997 rejects TLS 1.0 and 1.1 with a protocol-version alert before SMTP can begin.
#[tokio::test]
async fn obsolete_tls_versions_receive_protocol_version_alerts() {
    let certificate = test_certificate();
    for minor_version in [1, 2] {
        for implicit in [false, true] {
            let deliveries = RecordedDeliveries::default();
            let transport_security = if implicit {
                TransportSecurity::Implicit {
                    configuration: certificate.configuration.clone(),
                }
            } else {
                TransportSecurity::StartTls {
                    configuration: certificate.configuration.clone(),
                    required: true,
                }
            };
            let mut reader = start_secured_session(
                ClosureHandler(record_into(deliveries.clone())),
                AuthenticationPolicy::Optional,
                transport_security,
                Limits::default(),
            );
            if !implicit {
                read_mail_protocol_response_line(&mut reader).await;
                send_mail_protocol_command(&mut reader, "EHLO client").await;
                assert!(
                    send_mail_protocol_command(&mut reader, "STARTTLS")
                        .await
                        .starts_with("220")
                );
            }
            // Legacy ClientHello: version, random, empty session ID, one cipher, null compression.
            // Build the wire record directly because rustls clients cannot offer obsolete versions.
            let mut hello = vec![3, minor_version];
            hello.extend_from_slice(&[0; 32]);
            hello.extend_from_slice(&[0, 0, 2, 0xc0, 0x2f, 1, 0]);
            // Offer modern signature schemes so certificate selection cannot hide version refusal.
            hello.extend_from_slice(&[0, 10, 0, 13, 0, 6, 0, 4, 4, 3, 4, 1]);
            let mut handshake = vec![1, 0, 0, hello.len() as u8];
            handshake.extend_from_slice(&hello);
            let mut record = vec![22, 3, minor_version, 0, handshake.len() as u8];
            record.extend_from_slice(&handshake);
            reader.get_mut().write_all(&record).await.unwrap();
            let mut response = Vec::new();
            tokio::time::timeout(Duration::from_secs(2), reader.read_to_end(&mut response))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(response.len(), 7, "{response:?}");
            assert_eq!(response[0], 21, "TLS alert record");
            assert_eq!(
                &response[3..],
                &[0, 2, 2, 70],
                "fatal protocol_version alert"
            );
            assert!(deliveries.lock().unwrap().is_empty());
        }
    }
}

/// The handshake forgets everything negotiated in clear text, including the principal.
#[tokio::test]
async fn start_tls_resets_greeting_and_authentication() {
    let certificate = test_certificate();
    let mut reader = start_tls_session(
        &certificate,
        AuthenticationPolicy::Required,
        false,
        RecordedDeliveries::default(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    let credentials = plain_response("", TEST_USERNAME, TEST_PASSWORD);
    assert!(
        send_mail_protocol_command(&mut reader, &format!("AUTH PLAIN {credentials}"))
            .await
            .starts_with("235")
    );
    send_mail_protocol_command(&mut reader, "STARTTLS").await;
    let mut encrypted = connect_with_tls(&certificate.connector, reader.into_inner()).await;
    assert!(
        send_mail_protocol_command(&mut encrypted, "AUTH PLAIN")
            .await
            .starts_with("503 5.5.1 send EHLO first"),
        "the greeting is forgotten"
    );
    send_mail_protocol_command(&mut encrypted, "EHLO client").await;
    assert!(
        send_mail_protocol_command(&mut encrypted, "MAIL FROM:<app@example.test>")
            .await
            .starts_with("530 5.7.0 authentication required"),
        "the plaintext principal is forgotten"
    );
}

/// Commands pipelined behind STARTTLS are never executed: the connection closes instead.
#[tokio::test]
async fn commands_pipelined_after_start_tls_close_the_connection() {
    let certificate = test_certificate();
    let mut reader = start_tls_session(
        &certificate,
        AuthenticationPolicy::Optional,
        false,
        RecordedDeliveries::default(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    reader
        .get_mut()
        .write_all(b"STARTTLS\r\nMAIL FROM:<injected@example.test>\r\n")
        .await
        .unwrap();
    assert!(
        read_mail_protocol_response_line(&mut reader)
            .await
            .starts_with("421 4.7.0")
    );
    assert_eq!(read_mail_protocol_response_line(&mut reader).await, "");
}

/// Required TLS hides AUTH, refuses it with 538, and refuses mail with 530 until STARTTLS.
#[tokio::test]
async fn required_tls_gates_authentication_and_mail() {
    let certificate = test_certificate();
    let mut reader = start_tls_session(
        &certificate,
        AuthenticationPolicy::Optional,
        true,
        RecordedDeliveries::default(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    reader
        .get_mut()
        .write_all(b"EHLO client\r\n")
        .await
        .unwrap();
    let capabilities = read_mail_protocol_reply_lines(&mut reader).await;
    assert_eq!(
        capabilities.last().unwrap(),
        "250 STARTTLS",
        "{capabilities:?}"
    );
    assert!(!capabilities.iter().any(|line| line.contains("AUTH")));
    assert!(
        send_mail_protocol_command(&mut reader, "AUTH PLAIN")
            .await
            .starts_with("538 5.7.11")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "MAIL FROM:<app@example.test>")
            .await
            .starts_with("530 5.7.0 must issue a STARTTLS command first")
    );
    send_mail_protocol_command(&mut reader, "STARTTLS").await;
    let mut encrypted = connect_with_tls(&certificate.connector, reader.into_inner()).await;
    send_mail_protocol_command(&mut encrypted, "EHLO client").await;
    assert!(
        submit_test_message(&mut encrypted, "now allowed")
            .await
            .starts_with("250")
    );
}

/// STARTTLS is not implemented without a certificate and refused during a transaction.
#[tokio::test]
async fn start_tls_is_refused_when_unavailable_or_out_of_sequence() {
    let mut plaintext = start_session(
        ClosureHandler(record_into(RecordedDeliveries::default())),
        AuthenticationPolicy::Optional,
        Limits::default(),
    );
    read_mail_protocol_response_line(&mut plaintext).await;
    send_mail_protocol_command(&mut plaintext, "EHLO client").await;
    assert!(
        send_mail_protocol_command(&mut plaintext, "STARTTLS")
            .await
            .starts_with("502 5.5.1")
    );

    let certificate = test_certificate();
    let mut reader = start_tls_session(
        &certificate,
        AuthenticationPolicy::Optional,
        false,
        RecordedDeliveries::default(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    send_mail_protocol_command(&mut reader, "MAIL FROM:<app@example.test>").await;
    assert!(
        send_mail_protocol_command(&mut reader, "STARTTLS")
            .await
            .starts_with("503 5.5.1")
    );
}

/// STARTTLS has no argument, and a failed TLS negotiation closes without accepting plaintext mail.
#[tokio::test]
async fn start_tls_rejects_arguments_and_failed_handshakes() {
    let certificate = test_certificate();
    let deliveries = RecordedDeliveries::default();
    let mut reader = start_tls_session(
        &certificate,
        AuthenticationPolicy::Optional,
        false,
        deliveries.clone(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    assert!(
        send_mail_protocol_command(&mut reader, "STARTTLS extra")
            .await
            .starts_with("501 5.5.4")
    );
    assert!(
        send_mail_protocol_command(&mut reader, "STARTTLS")
            .await
            .starts_with("220")
    );
    reader
        .get_mut()
        .write_all(b"MAIL FROM:<injected@example.test>\r\n")
        .await
        .unwrap();
    let mut remaining = Vec::new();
    reader.read_to_end(&mut remaining).await.unwrap();
    assert!(!remaining.windows(3).any(|window| window == b"250"));
    assert!(deliveries.lock().unwrap().is_empty());
}

/// STARTTLS resets the failure budget along with the rest of the pre-TLS SMTP state.
#[tokio::test]
async fn start_tls_resets_authentication_failure_budget() {
    let certificate = test_certificate();
    let mut reader = start_tls_session(
        &certificate,
        AuthenticationPolicy::Optional,
        false,
        RecordedDeliveries::default(),
    );
    read_mail_protocol_response_line(&mut reader).await;
    send_mail_protocol_command(&mut reader, "EHLO client").await;
    let invalid_credentials = plain_response("", TEST_USERNAME, "wrong password");
    for _ in 0..2 {
        assert!(
            send_mail_protocol_command(&mut reader, &format!("AUTH PLAIN {invalid_credentials}"))
                .await
                .starts_with("535")
        );
    }
    assert!(
        send_mail_protocol_command(&mut reader, "STARTTLS")
            .await
            .starts_with("220")
    );
    let mut encrypted = connect_with_tls(&certificate.connector, reader.into_inner()).await;
    send_mail_protocol_command(&mut encrypted, "EHLO client").await;
    for _ in 0..2 {
        assert!(
            send_mail_protocol_command(
                &mut encrypted,
                &format!("AUTH PLAIN {invalid_credentials}")
            )
            .await
            .starts_with("535"),
            "the pre-TLS attempts must not exhaust the post-TLS failure budget"
        );
    }
    assert!(
        send_mail_protocol_command(&mut encrypted, &format!("AUTH PLAIN {invalid_credentials}"))
            .await
            .starts_with("421")
    );
}

/// Implicit TLS completes the handshake before the greeting and never offers STARTTLS.
#[tokio::test]
async fn implicit_tls_handshakes_before_the_greeting() {
    let certificate = test_certificate();
    let reader = start_secured_session(
        ClosureHandler(record_into(RecordedDeliveries::default())),
        AuthenticationPolicy::Optional,
        TransportSecurity::Implicit {
            configuration: certificate.configuration.clone(),
        },
        Limits::default(),
    );
    let mut encrypted = connect_with_tls(&certificate.connector, reader.into_inner()).await;
    assert!(
        read_mail_protocol_response_line(&mut encrypted)
            .await
            .starts_with("220")
    );
    encrypted
        .get_mut()
        .write_all(b"EHLO client\r\n")
        .await
        .unwrap();
    let capabilities = read_mail_protocol_reply_lines(&mut encrypted).await;
    assert!(
        !capabilities.iter().any(|line| line.ends_with("STARTTLS")),
        "{capabilities:?}"
    );
    assert!(capabilities.contains(&"250 AUTH PLAIN LOGIN".to_owned()));
}

/// QUIT returns its reply and a valid TLS close alert, so the client observes clean EOF.
#[tokio::test]
async fn implicit_tls_quit_closes_cleanly() {
    let certificate = test_certificate();
    let reader = start_secured_session(
        ClosureHandler(record_into(RecordedDeliveries::default())),
        AuthenticationPolicy::Optional,
        TransportSecurity::Implicit {
            configuration: certificate.configuration.clone(),
        },
        Limits::default(),
    );
    let mut encrypted = connect_with_tls(&certificate.connector, reader.into_inner()).await;
    read_mail_protocol_response_line(&mut encrypted).await;
    assert!(
        send_mail_protocol_command(&mut encrypted, "QUIT")
            .await
            .starts_with("221 2.0.0")
    );

    let mut remainder = Vec::new();
    assert_eq!(encrypted.read_to_end(&mut remainder).await.unwrap(), 0);
    assert!(remainder.is_empty());
}

/// Session lifetime expiry sends 421, then a TLS close alert that produces clean EOF.
#[tokio::test(start_paused = true)]
async fn implicit_tls_session_expiry_closes_cleanly() {
    let certificate = test_certificate();
    let reader = start_secured_session(
        ClosureHandler(record_into(RecordedDeliveries::default())),
        AuthenticationPolicy::Optional,
        TransportSecurity::Implicit {
            configuration: certificate.configuration.clone(),
        },
        Limits {
            maximum_session_duration: Duration::from_secs(1),
            ..Limits::default()
        },
    );
    let mut encrypted = connect_with_tls(&certificate.connector, reader.into_inner()).await;
    read_mail_protocol_response_line(&mut encrypted).await;
    assert!(
        read_mail_protocol_response_line(&mut encrypted)
            .await
            .starts_with("421 4.4.2 session time limit reached")
    );

    let mut remainder = Vec::new();
    assert_eq!(encrypted.read_to_end(&mut remainder).await.unwrap(), 0);
    assert!(remainder.is_empty());
}

/// Submit through implicit TLS and return the captured message's generated Received field.
async fn implicit_tls_received_header(authenticate: bool) -> String {
    let certificate = test_certificate();
    let (message_sender, mut message_receiver) = tokio::sync::mpsc::channel(1);
    let reader = start_secured_session(
        ClosureHandler(move |message, _| {
            let message_sender = message_sender.clone();
            async move {
                message_sender
                    .send(message)
                    .await
                    .map_err(std::io::Error::other)
            }
        }),
        if authenticate {
            AuthenticationPolicy::Required
        } else {
            AuthenticationPolicy::Optional
        },
        TransportSecurity::Implicit {
            configuration: certificate.configuration.clone(),
        },
        Limits::default(),
    );
    let mut encrypted = connect_with_tls(&certificate.connector, reader.into_inner()).await;
    read_mail_protocol_response_line(&mut encrypted).await;
    send_mail_protocol_command(&mut encrypted, "EHLO client").await;
    if authenticate {
        let credentials = plain_response("", TEST_USERNAME, TEST_PASSWORD);
        assert!(
            send_mail_protocol_command(&mut encrypted, &format!("AUTH PLAIN {credentials}"))
                .await
                .starts_with("235")
        );
    }
    assert!(
        submit_test_message(&mut encrypted, "tls trace")
            .await
            .starts_with("250")
    );
    message_receiver.recv().await.unwrap().facts.headers["received"][0].clone()
}

/// Received identifies encrypted sessions as ESMTPS and encrypted authenticated sessions as ESMTPSA.
#[tokio::test]
async fn implicit_tls_received_protocol_tracks_authentication() {
    for (authenticate, protocol) in [(false, "ESMTPS"), (true, "ESMTPSA")] {
        let received = implicit_tls_received_header(authenticate).await;
        assert!(
            received.contains(&format!("with {protocol};")),
            "{received}"
        );
    }
}

/// Generated Received fields distinguish SMTP, ESMTP, and authenticated ESMTP sessions.
#[tokio::test]
async fn plaintext_received_protocol_tracks_greeting_and_authentication() {
    for (extended, authenticate, protocol) in [
        (false, false, "SMTP"),
        (true, false, "ESMTP"),
        (true, true, "ESMTPA"),
    ] {
        let (message_sender, mut message_receiver) = tokio::sync::mpsc::channel(1);
        let handler = ClosureHandler(move |message, _| {
            let message_sender = message_sender.clone();
            async move {
                message_sender
                    .send(message)
                    .await
                    .map_err(std::io::Error::other)
            }
        });
        let mut reader = start_session(handler, AuthenticationPolicy::Optional, Limits::default());
        read_mail_protocol_response_line(&mut reader).await;
        send_mail_protocol_command(
            &mut reader,
            if extended {
                "EHLO client"
            } else {
                "HELO client"
            },
        )
        .await;
        if authenticate {
            let credentials = plain_response("", TEST_USERNAME, TEST_PASSWORD);
            assert!(
                send_mail_protocol_command(&mut reader, &format!("AUTH PLAIN {credentials}"))
                    .await
                    .starts_with("235")
            );
        }
        assert!(
            submit_test_message(&mut reader, "plaintext trace")
                .await
                .starts_with("250")
        );
        let message = message_receiver.recv().await.unwrap();
        assert!(
            message.facts.headers["received"][0].contains(&format!("with {protocol};")),
            "{}",
            message.facts.headers["received"][0]
        );
    }
}

/// A client that never starts the implicit handshake is disconnected after the handshake limit.
#[tokio::test(start_paused = true)]
async fn stalled_implicit_handshakes_time_out() {
    let certificate = test_certificate();
    let limits = Limits {
        tls_handshake_timeout: Duration::from_secs(5),
        ..Limits::default()
    };
    let mut reader = start_secured_session(
        ClosureHandler(record_into(RecordedDeliveries::default())),
        AuthenticationPolicy::Optional,
        TransportSecurity::Implicit {
            configuration: certificate.configuration.clone(),
        },
        limits,
    );
    let started = tokio::time::Instant::now();
    let mut received = Vec::new();
    let read = reader.read_to_end(&mut received).await.unwrap();
    assert_eq!(read, 0, "no greeting is sent before the handshake");
    assert!(started.elapsed() >= limits.tls_handshake_timeout);
}

/// Credentials presented over TLS reach the handler marked as encrypted.
#[tokio::test]
async fn credentials_over_tls_are_marked_encrypted() {
    let certificate = test_certificate();
    let received = ReceivedAuthentications::default();
    let reader = start_secured_session(
        RulesHandler(received.clone()),
        AuthenticationPolicy::Optional,
        TransportSecurity::Implicit {
            configuration: certificate.configuration.clone(),
        },
        Limits::default(),
    );
    let mut encrypted = connect_with_tls(&certificate.connector, reader.into_inner()).await;
    read_mail_protocol_response_line(&mut encrypted).await;
    send_mail_protocol_command(&mut encrypted, "EHLO client").await;
    assert!(
        send_mail_protocol_command(
            &mut encrypted,
            &format!("AUTH PLAIN {}", plain_response("", "tls-only", "secret"))
        )
        .await
        .starts_with("235")
    );
    assert_eq!(
        received.lock().unwrap().as_slice(),
        [(sandpost_core::SmtpAuthenticationMechanism::Plain, true)]
    );
}
