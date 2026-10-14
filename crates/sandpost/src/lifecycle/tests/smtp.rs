//! SMTP access, live listener changes, and shared persistence through the runtime.

use super::*;

/// Encode an AUTH PLAIN initial response as standard padded base64.
fn plain_response(username: &str, password: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = format!("\0{username}\0{password}").into_bytes();
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

/// Send one SMTP command and return the final line of its possibly multi-line reply.
async fn smtp_command(
    stream: &mut tokio::io::BufReader<tokio::net::TcpStream>,
    command: &str,
) -> String {
    use tokio::io::AsyncBufReadExt;
    stream
        .get_mut()
        .write_all(command.as_bytes())
        .await
        .unwrap();
    loop {
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        if line.as_bytes().get(3) != Some(&b'-') {
            return line;
        }
    }
}

/// Submit one message over EHLO, optionally authenticating first, and return the reply to
/// the first command that does not succeed or to the final DATA terminator.
async fn submit_with_credentials(
    address: SocketAddr,
    credentials: Option<(&str, &str)>,
    subject: &str,
) -> String {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut stream = BufReader::new(tokio::net::TcpStream::connect(address).await.unwrap());
    let mut greeting = String::new();
    stream.read_line(&mut greeting).await.unwrap();
    assert!(greeting.starts_with("220"));
    assert!(
        smtp_command(&mut stream, "EHLO example.test\r\n")
            .await
            .starts_with("250")
    );
    if let Some((username, password)) = credentials {
        let response = smtp_command(
            &mut stream,
            &format!("AUTH PLAIN {}\r\n", plain_response(username, password)),
        )
        .await;
        if !response.starts_with("235") {
            return response;
        }
    }
    for (command, expected) in [
        ("MAIL FROM:<app@example.test>\r\n".to_owned(), "250"),
        ("RCPT TO:<reader@example.test>\r\n".to_owned(), "250"),
        ("DATA\r\n".to_owned(), "354"),
        (format!("Subject: {subject}\r\n\r\nbody\r\n.\r\n"), "250"),
    ] {
        let response = smtp_command(&mut stream, &command).await;
        if !response.starts_with(expected) {
            return response;
        }
    }
    "250 message accepted".into()
}

/// Complete first-run setup on a started runtime and return an Owner session.
async fn owner_session(running: &RunningSandpost) -> sandpost_application::SessionUser {
    let instance = running.application.instance();
    let secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
    instance
        .setup(sandpost_application::SetupRequest {
            bootstrap_secret: secret,
            name: "Runtime Owner".into(),
            email: "owner@example.test".into(),
            password: "runtime test password".into(),
        })
        .await
        .unwrap();
    instance
        .login(sandpost_application::LoginRequest {
            email: "owner@example.test".into(),
            password: "runtime test password".into(),
        })
        .await
        .unwrap()
        .user
}

/// One global listener accepts mail from every SMTP access, refuses crossed credentials,
/// keeps local unauthenticated mail, and shares one SQL/outbox/index.
#[tokio::test]
async fn smtp_accesses_deliver_through_one_global_listener() {
    let (configuration, _directory) = test_configuration();
    let running = SandpostRuntime::start(configuration).await.unwrap();
    assert!(!running.mail_authentication_required, "loopback stays open");
    let owner = owner_session(&running).await;
    let authorized = running.application.authorized(&owner);
    let issue = |name: &'static str| {
        let authorized = authorized.clone();
        async move {
            authorized
                .create_smtp_access(sandpost_application::NewSmtpAccessRequest {
                    name: name.into(),
                    username: None,
                    requires_encryption: false,
                    allowed_mechanisms: sandpost_core::SmtpAuthenticationMechanism::ALL.to_vec(),
                })
                .await
                .unwrap()
        }
    };
    let laravel = issue("Laravel").await;
    let pipeline = issue("CI Pipeline").await;
    let address = running.mail_listen_address.unwrap();
    for (credentials, subject) in [
        (
            Some((laravel.access.username.as_str(), laravel.password.as_str())),
            "laravel",
        ),
        (
            Some((
                pipeline.access.username.as_str(),
                pipeline.password.as_str(),
            )),
            "pipeline",
        ),
        (None, "anonymous"),
    ] {
        let response = submit_with_credentials(address, credentials, subject).await;
        assert!(response.starts_with("250"), "{subject}: {response}");
    }
    let refused = submit_with_credentials(
        address,
        Some((pipeline.access.username.as_str(), laravel.password.as_str())),
        "crossed",
    )
    .await;
    assert!(refused.starts_with("535"), "{refused}");

    let search = running.application.search_coordinator().clone();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let status = search.status().await.unwrap();
        if status.pending_operations == 0 && status.latest_sequence >= 3 {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut subjects: Vec<String> = authorized
        .query_messages(sandpost_application::QueryOptions::default())
        .await
        .unwrap()
        .into_iter()
        .map(|message| message.subject)
        .collect();
    subjects.sort();
    assert_eq!(subjects, ["anonymous", "laravel", "pipeline"]);
    running.shutdown().await.unwrap();
}

/// The persisted server address moves at runtime, and a required policy refuses
/// unauthenticated mail even on loopback while revoked credentials stop working.
#[tokio::test]
async fn global_listener_moves_at_runtime_and_enforces_required_authentication() {
    let (mut configuration, _directory) = test_configuration();
    configuration.mail_listen_address = None;
    configuration.mail_authentication = crate::configuration::MailAuthenticationSetting::Required;
    std::fs::create_dir_all(&configuration.data_directory).unwrap();
    {
        let storage = sandpost_storage_sqlite::SqliteStorage::open(&configuration.database_path)
            .await
            .unwrap();
        sandpost_storage::SmtpServerStorage::save_smtp_listen_address(
            &storage,
            Some("127.0.0.1:0".parse().unwrap()),
        )
        .await
        .unwrap();
    }
    let running = SandpostRuntime::start(configuration).await.unwrap();
    assert!(running.mail_authentication_required);
    let owner = owner_session(&running).await;
    let authorized = running.application.authorized(&owner);
    let first_address = running.mail_listen_address.unwrap();
    let free_address = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let moved = authorized
        .save_smtp_server(sandpost_application::SmtpServerUpdate {
            listen_address: Some(free_address),
        })
        .await
        .unwrap();
    let listener = moved.listener.unwrap();
    assert_eq!(
        listener.source,
        sandpost_application::MailListenerSource::Database
    );
    assert!(listener.authentication_required);
    let address = listener.bound_address.unwrap();
    assert_eq!(address, free_address);
    assert_ne!(address, first_address);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::net::TcpStream::connect(first_address).await.is_ok() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the previous listener must release its port"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let anonymous = submit_with_credentials(address, None, "anonymous").await;
    assert!(anonymous.starts_with("530"), "{anonymous}");
    let issued = authorized
        .create_smtp_access(sandpost_application::NewSmtpAccessRequest {
            name: "CI Pipeline".into(),
            username: None,
            requires_encryption: false,
            allowed_mechanisms: sandpost_core::SmtpAuthenticationMechanism::ALL.to_vec(),
        })
        .await
        .unwrap();
    let username = issued.access.username.clone();
    let accepted =
        submit_with_credentials(address, Some((&username, &issued.password)), "ci").await;
    assert!(accepted.starts_with("250"), "{accepted}");
    authorized
        .update_smtp_access(
            issued.access.identifier,
            sandpost_application::SmtpAccessUpdate {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let disabled =
        submit_with_credentials(address, Some((&username, &issued.password)), "disabled").await;
    assert!(disabled.starts_with("535"), "{disabled}");
    let tls_only = authorized
        .create_smtp_access(sandpost_application::NewSmtpAccessRequest {
            name: "TLS only".into(),
            username: None,
            requires_encryption: true,
            allowed_mechanisms: sandpost_core::SmtpAuthenticationMechanism::ALL.to_vec(),
        })
        .await
        .unwrap();
    let refused = submit_with_credentials(
        address,
        Some((&tls_only.access.username, &tls_only.password)),
        "clear text",
    )
    .await;
    assert!(refused.starts_with("538"), "{refused}");
    running.shutdown().await.unwrap();
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
}
