//! IMAP listener disablement, re-enablement, and persisted runtime data.

use super::*;

/// Configure a local runtime from a TOML file and an isolated environment lookup.
fn configuration(
    configuration_path: &std::path::Path,
    directory: &std::path::Path,
    imap_enabled: Option<bool>,
) -> Configuration {
    Configuration::from_environment_values(|name| match name {
        "SANDPOST_CONFIG" => Some(configuration_path.to_string_lossy().into_owned()),
        "SANDPOST_DATA_DIRECTORY" => Some(directory.to_string_lossy().into_owned()),
        "SANDPOST_HTTP_PORT" => Some("0".into()),
        "SANDPOST_IMAP_ENABLED" => imap_enabled.map(|enabled| enabled.to_string()),
        _ => None,
    })
    .unwrap()
}

/// Complete setup and return the Owner's session identity.
async fn owner_session(running: &RunningSandpost) -> sandpost_application::SessionUser {
    let instance = running.application.instance();
    instance
        .setup(sandpost_application::SetupRequest {
            bootstrap_secret: instance.bootstrap_secret_for_startup().unwrap().unwrap(),
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

/// Disabled IMAP leaves its configured port free while HTTP and persisted SMTP operate;
/// account data created before disablement survives restart with IMAP enabled again.
#[tokio::test]
async fn disabled_imap_keeps_services_and_data_available_across_restart() {
    let directory = TestDirectory(
        std::env::temp_dir().join(format!("sandpost-runtime-imap-{}", uuid::Uuid::new_v4())),
    );
    std::fs::create_dir_all(&directory.0).unwrap();
    let imap_reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let imap_port = imap_reservation.local_addr().unwrap().port();
    drop(imap_reservation);
    let configuration_path = directory.0.join("runtime.toml");
    std::fs::write(
        &configuration_path,
        format!(
            "[http]\nbind_address = \"127.0.0.1\"\nport = 1\n\n[imap]\nenabled = true\nbind_address = \"127.0.0.1\"\nport = {imap_port}\n"
        ),
    )
    .unwrap();
    {
        let storage =
            sandpost_storage_sqlite::SqliteStorage::open(&directory.0.join("sandpost.sqlite3"))
                .await
                .unwrap();
        sandpost_storage::SmtpServerStorage::save_smtp_listen_address(
            &storage,
            Some("127.0.0.1:0".parse().unwrap()),
        )
        .await
        .unwrap();
    }

    let enabled =
        SandpostRuntime::start(configuration(&configuration_path, &directory.0, Some(true)))
            .await
            .unwrap();
    assert_eq!(
        enabled
            .imap
            .as_ref()
            .unwrap()
            .listen_address
            .unwrap()
            .port(),
        imap_port
    );
    let owner = owner_session(&enabled).await;
    let account = enabled
        .application
        .authorized(&owner)
        .create_imap_account(sandpost_application::NewImapAccountRequest {
            name: "Persisted account".into(),
            username: None,
            mirror_views: true,
            mode: Some(sandpost_core::ImapAccountMode::Independent),
            linked_views: Vec::new(),
        })
        .await
        .unwrap();
    let account_identifier = account.account.account.identifier;
    enabled.shutdown().await.unwrap();

    let imap_reservation = tokio::net::TcpListener::bind(("127.0.0.1", imap_port))
        .await
        .expect("the prior IMAP listener must release its port");
    let disabled = SandpostRuntime::start(configuration(
        &configuration_path,
        &directory.0,
        Some(false),
    ))
    .await
    .unwrap();
    assert!(disabled.imap.is_none());
    assert!(
        disabled.mail_listen_address.is_some(),
        "persisted SMTP binds while IMAP is disabled"
    );

    let mut http = tokio::net::TcpStream::connect(disabled.web_listen_address)
        .await
        .unwrap();
    http.write_all(
        b"GET /api/v1/instance HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await
    .unwrap();
    let mut http_response = String::new();
    tokio::time::timeout(
        Duration::from_secs(2),
        http.read_to_string(&mut http_response),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        http_response.starts_with("HTTP/1.1 200 OK\r\n"),
        "{http_response}"
    );

    let mut smtp = tokio::net::TcpStream::connect(disabled.mail_listen_address.unwrap())
        .await
        .unwrap();
    let mut smtp_greeting = [0; 128];
    let greeting_length =
        tokio::time::timeout(Duration::from_secs(2), smtp.read(&mut smtp_greeting))
            .await
            .unwrap()
            .unwrap();
    assert!(smtp_greeting[..greeting_length].starts_with(b"220"));
    let owner = disabled
        .application
        .instance()
        .login(sandpost_application::LoginRequest {
            email: "owner@example.test".into(),
            password: "runtime test password".into(),
        })
        .await
        .unwrap()
        .user;
    assert!(
        disabled
            .application
            .authorized(&owner)
            .list_imap_accounts()
            .await
            .unwrap()
            .iter()
            .any(|account| account.account.identifier == account_identifier)
    );
    disabled.shutdown().await.unwrap();
    drop(imap_reservation);

    let restarted =
        SandpostRuntime::start(configuration(&configuration_path, &directory.0, Some(true)))
            .await
            .unwrap();
    assert_eq!(
        restarted.imap.as_ref().unwrap().listen_address,
        Some(format!("127.0.0.1:{imap_port}").parse().unwrap())
    );
    assert!(restarted.mail_listen_address.is_some());
    let owner = restarted
        .application
        .instance()
        .login(sandpost_application::LoginRequest {
            email: "owner@example.test".into(),
            password: "runtime test password".into(),
        })
        .await
        .unwrap()
        .user;
    let accounts = restarted
        .application
        .authorized(&owner)
        .list_imap_accounts()
        .await
        .unwrap();
    assert!(accounts.iter().any(|account| {
        account.account.identifier == account_identifier
            && account.account.name == "Persisted account"
    }));
    restarted.shutdown().await.unwrap();
}

/// Bind all listeners on IPv6 loopback and verify the IMAP socket accepts clients.
#[tokio::test]
async fn runtime_listeners_bind_ipv6_loopback() {
    let directory = TestDirectory(
        std::env::temp_dir().join(format!("sandpost-runtime-ipv6-{}", uuid::Uuid::new_v4())),
    );
    let configuration = Configuration::from_values(|name| match name {
        "SANDPOST_DATA_DIRECTORY" => Some(directory.0.to_string_lossy().into_owned()),
        "SANDPOST_HTTP_BIND_ADDRESS" | "SANDPOST_SMTP_BIND_ADDRESS" => Some("::1".into()),
        "SANDPOST_HTTP_PORT" | "SANDPOST_SMTP_PORT" => Some("0".into()),
        "SANDPOST_IMAP_BIND_ADDRESS" => Some("::1".into()),
        "SANDPOST_IMAP_PORT" => Some("0".into()),
        _ => None,
    })
    .unwrap();
    let running = SandpostRuntime::start(configuration).await.unwrap();
    assert!(running.web_listen_address.is_ipv6());
    assert!(running.mail_listen_address.unwrap().is_ipv6());
    let address = running.imap.as_ref().unwrap().listen_address.unwrap();
    assert!(address.is_ipv6() && address.ip().is_loopback(), "{address}");
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let mut greeting = [0; 256];
    let length = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut greeting))
        .await
        .unwrap()
        .unwrap();
    assert!(greeting[..length].starts_with(b"* OK"));
    running.shutdown().await.unwrap();
}
