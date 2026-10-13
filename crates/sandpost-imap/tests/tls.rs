//! TLS conformance: STARTTLS (RFC 3501 section 6.2.1, RFC 9051 section 6.2.1), LOGINDISABLED,
//! and implicit TLS (RFC 8314 section 3.2).
mod support;

use sandpost_imap::{TlsConfiguration, TransportSecurity};
use std::sync::Arc;
use support::{RunningServer, backend::PASSWORD, client::TestClient, test_limits};
use tokio::net::TcpStream;
use tokio_rustls::{
    TlsConnector,
    rustls::{ClientConfig, RootCertStore, crypto::ring, pki_types::ServerName},
};

/// A self-signed certificate for localhost and a client trusting only it.
fn certificate() -> (TlsConfiguration, TlsConnector) {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let configuration = TlsConfiguration::from_pem(
        certified.cert.pem().as_bytes(),
        certified.signing_key.serialize_pem().as_bytes(),
    )
    .unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(certified.cert.der().clone()).unwrap();
    let connector = TlsConnector::from(Arc::new(
        ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ));
    (configuration, connector)
}

/// Required STARTTLS announces LOGINDISABLED, refuses passwords in clear text, discards input
/// pipelined after STARTTLS, and allows login once encrypted.
#[tokio::test]
async fn required_start_tls_disables_login_until_upgraded() {
    let (configuration, connector) = certificate();
    let server = RunningServer::start_with(
        test_limits(),
        TransportSecurity::StartTls {
            configuration,
            required: true,
        },
    )
    .await;
    let mut client = server.connect().await;
    let capabilities = client.command("t1", "CAPABILITY").await;
    assert_eq!(
        capabilities[0],
        "* CAPABILITY IMAP4rev1 STARTTLS LOGINDISABLED LITERAL- ID IDLE NAMESPACE UIDPLUS MOVE UNSELECT CHILDREN\r\n"
    );
    assert_eq!(
        client
            .status_of("t2", &format!("LOGIN developer \"{PASSWORD}\""))
            .await,
        "t2 NO [PRIVACYREQUIRED] LOGIN requires TLS; use STARTTLS first\r\n"
    );
    assert_eq!(
        client.status_of("t3", "AUTHENTICATE PLAIN AGEAYg==").await,
        "t3 NO [PRIVACYREQUIRED] Authentication requires TLS; use STARTTLS first\r\n"
    );
    client.send(b"t4 STARTTLS\r\nt5 NOOP\r\n").await;
    assert_eq!(
        client.read_response().await.unwrap(),
        "t4 OK Begin TLS negotiation now\r\n"
    );
    let stream = client.reader.into_inner();
    let encrypted = connector
        .connect(ServerName::try_from("localhost").unwrap(), stream)
        .await
        .unwrap();
    let mut client = TestClient::new(encrypted);
    let capabilities = client.command("t6", "CAPABILITY").await;
    assert_eq!(
        capabilities,
        [
            "* CAPABILITY IMAP4rev1 AUTH=PLAIN SASL-IR LITERAL- ID IDLE NAMESPACE UIDPLUS MOVE UNSELECT CHILDREN\r\n",
            "t6 OK CAPABILITY completed\r\n"
        ],
        "the pipelined t5 was discarded"
    );
    assert_eq!(
        client.status_of("t7", "STARTTLS").await,
        "t7 BAD STARTTLS is not available\r\n"
    );
    client.login().await;
    server.stop().await;
}

/// Optional STARTTLS still allows login in clear text.
#[tokio::test]
async fn optional_start_tls_allows_plain_login() {
    let (configuration, _) = certificate();
    let server = RunningServer::start_with(
        test_limits(),
        TransportSecurity::StartTls {
            configuration,
            required: false,
        },
    )
    .await;
    let mut client = server.connect().await;
    let capabilities = client.command("o1", "CAPABILITY").await;
    assert!(
        capabilities[0].contains("STARTTLS AUTH=PLAIN"),
        "{capabilities:?}"
    );
    client.login().await;
    server.stop().await;
}

/// Implicit TLS performs the handshake before the greeting and never offers STARTTLS.
#[tokio::test]
async fn implicit_tls_greets_after_handshake() {
    let (configuration, connector) = certificate();
    let server =
        RunningServer::start_with(test_limits(), TransportSecurity::Implicit { configuration })
            .await;
    let stream = TcpStream::connect(server.address).await.unwrap();
    let encrypted = connector
        .connect(ServerName::try_from("localhost").unwrap(), stream)
        .await
        .unwrap();
    let mut client = TestClient::new(encrypted);
    let greeting = client.greeting().await;
    assert!(!greeting.contains("STARTTLS"), "{greeting}");
    assert!(greeting.contains("AUTH=PLAIN"), "{greeting}");
    assert_eq!(
        client.status_of("i1", "STARTTLS").await,
        "i1 BAD STARTTLS is not available\r\n"
    );
    client.login().await;
    server.stop().await;
}

/// Implicit TLS never sends plaintext protocol replies, even when refusing excess connections.
#[tokio::test]
async fn implicit_tls_connection_limit_sends_no_plaintext() {
    let (configuration, _) = certificate();
    let mut limits = test_limits();
    limits.maximum_connection_count = 0;
    let server =
        RunningServer::start_with(limits, TransportSecurity::Implicit { configuration }).await;
    let stream = TcpStream::connect(server.address).await.unwrap();
    let mut client = TestClient::new(stream);
    client.expect_closed().await;
    server.stop().await;
}
