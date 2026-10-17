use crate::{
    Result,
    http::HttpClient,
    mail,
    progress::Progress,
    server::{SECRET_SENTINEL, ServerConfiguration},
};
use serde_json::{Value, json};
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use std::{
    fs,
    io::{BufRead, BufReader},
    net::{TcpListener, TcpStream},
    path::Path,
    thread,
    time::{Duration, Instant},
};

const RAW_MESSAGE: &[u8] = b"From: Dev <dev@SENDER.TEST>\r\nTo: user@RECIPIENT.TEST\r\nSubject: Sand Post smoke\r\nX-App: bravo\r\nX-App: second\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHello from SMTP.\r\n.dot line\r\n";

/// Exercise the real server through ingestion, authorization, protocols, maintenance, and restart.
pub async fn run(binary: &Path, frontend: Option<&Path>) -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("sandpost-smoke-")
        .tempdir()?;
    let mut progress = Progress::new();
    progress.start("Prepare isolated smoke-test configuration");
    let configuration = ServerConfiguration::new(binary, directory.path())?;
    let mut client = HttpClient {
        port: configuration.web_port,
        session_token: None,
    };
    progress.start("Start server and verify listeners and startup logs");
    let process = configuration.start(&client)?;
    check_startup_summary(
        &configuration.stdout()?,
        &configuration.database_path(),
        configuration.web_port,
        configuration.mail_port,
        configuration.imap_port,
        1,
    );
    check_logs(&configuration, &[])?;
    if let Some(frontend) = frontend {
        progress.start("Verify embedded frontend assets and routes");
        check_frontend(&client, frontend)?;
    }
    progress.start("Create owner account and verify login");
    assert_eq!(
        client.get_json("/api/v1/instance")?,
        json!({"initialized": false, "setupKeyRequired": true})
    );
    let output = configuration.stdout()?;
    let secret = output
        .split_once("Bootstrap secret:")
        .ok_or("startup omitted bootstrap secret")?
        .1
        .split_whitespace()
        .next()
        .ok_or("empty bootstrap secret")?;
    assert!(secret.starts_with("SP-"));
    let authorization = client.post_json("/api/v1/setup/authorize", json!({"setupKey": secret}))?;
    assert_ne!(authorization["authorization"], secret);
    assert_eq!(
        client.get_json("/api/v1/instance")?,
        json!({"initialized": false, "setupKeyRequired": true})
    );
    let credentials =
        json!({"email": "owner@example.test", "password": "smoke test owner password"});
    client.post_json("/api/v1/setup", json!({"bootstrapSecret": authorization["authorization"], "name": "Smoke Owner", "email": credentials["email"], "password": credentials["password"]}))?;
    assert_eq!(
        client.get_json("/api/v1/instance")?,
        json!({"initialized": true, "setupKeyRequired": false})
    );
    client.session_token = Some(text(
        &client.post_json("/api/v1/login", credentials.clone())?,
        "token",
    )?);
    let mut events = client.events()?;
    assert!(events.read_event()?.contains("event: ready"));
    progress.start("Deliver SMTP mail and verify SSE, MIME, and raw message");
    mail::send_mail(
        configuration.mail_port,
        "dev@sender.test",
        "user@recipient.test",
        RAW_MESSAGE,
        None,
    )?;
    assert!(events.read_event()?.contains("event: message"));
    let messages = wait_for_messages(&client, 1)?;
    assert_eq!(messages[0]["subject"], "Sand Post smoke");
    let message_identifier = text(&messages[0], "id")?;
    let detail = client.get_json(&format!("/api/v1/messages/{message_identifier}"))?;
    assert_eq!(detail["facts"]["from"][0]["domain"], "sender.test");
    assert_eq!(
        detail["facts"]["headers"]["x-app"],
        json!(["bravo", "second"])
    );
    let downloaded = client.request(
        "GET",
        &format!("/api/v1/messages/{message_identifier}/raw"),
        None,
    )?;
    assert_eq!(downloaded.status, 200);
    let captured_message = downloaded.body;
    assert!(captured_message.ends_with(RAW_MESSAGE));
    assert!(captured_message.starts_with(b"Return-Path: <dev@sender.test>\r\nReceived: "));
    progress.start("Verify SQLite ingestion, migrations, and search indexing");
    check_ingested_database(&configuration.database_path()).await?;
    assert_eq!(
        client.get_json("/api/v1/search/status")?["pending_operations"],
        0
    );

    progress.start("Issue SMTP credentials and verify password storage");
    let issued_access =
        client.post_json("/api/v1/smtp/accesses", json!({"name": "Smoke access"}))?;
    let smtp_username = text(&issued_access["access"], "username")?;
    let smtp_password = text(&issued_access, "password")?;
    assert_eq!(
        issued_access["server"]["listener"]["bound_address"],
        format!("127.0.0.1:{}", configuration.mail_port)
    );
    check_listed_credentials(
        &client.get_json("/api/v1/smtp/accesses")?,
        &smtp_username,
        &smtp_password,
    );
    check_password_hash(
        &configuration.database_path(),
        "select password_hash from smtp_accesses",
        &smtp_password,
    )
    .await?;
    mail::reject_smtp_password(configuration.mail_port, &smtp_username)?;
    assert_eq!(
        client.get_json(&format!("/api/v1/messages?before={}", messages[0]["seq"]))?,
        json!([])
    );
    progress.start("Verify search, rebuild, and live-writer locking");
    assert_eq!(
        client.get_json("/api/v1/search?q=subject%20%3D%3D%20%22Sand%20Post%20smoke%22")?[0]["id"],
        message_identifier
    );
    assert_eq!(
        client.post_json("/api/v1/search/rebuild", json!({}))?["pending_operations"],
        0
    );
    assert_eq!(
        client.get_json("/api/v1/messages")?[0]["id"],
        message_identifier
    );
    let (status, output, errors) = configuration.complete(
        configuration.command().args(["search", "status"]),
        Duration::from_secs(10),
    )?;
    assert!(
        status.success() && output.contains("pending=0"),
        "{output}{errors}"
    );
    let (status, _, errors) = configuration.complete(
        configuration.command().args(["search", "rebuild"]),
        Duration::from_secs(10),
    )?;
    assert!(
        !status.success() && errors.contains("search writer is already running"),
        "{errors}"
    );

    progress.start("Verify IMAP auth, mailboxes, FETCH, SEARCH, STORE, IDLE, and MOVE");
    let issued_imap = client.post_json(
        "/api/v1/imap/accounts",
        json!({"name": "Smoke IMAP", "mode": "independent"}),
    )?;
    let imap_username = text(&issued_imap["account"], "username")?;
    let imap_password = text(&issued_imap, "password")?;
    assert_eq!(
        issued_imap["server"],
        json!({"enabled": true, "host": "127.0.0.1", "port": configuration.imap_port, "security": "none", "listen_address": format!("127.0.0.1:{}", configuration.imap_port)})
    );
    check_listed_credentials(
        &client.get_json("/api/v1/imap/accounts")?,
        &imap_username,
        &imap_password,
    );
    check_password_hash(
        &configuration.database_path(),
        "select password_hash from imap_accounts",
        &imap_password,
    )
    .await?;
    let validity = mail::check_imap(
        configuration.imap_port,
        &imap_username,
        &imap_password,
        &captured_message,
        configuration.mail_port,
    )?;
    wait_for_messages(&client, 2)?;
    check_logs(&configuration, &[&imap_password])?;
    drop(events);
    process.stop()?;

    progress.start("Restart server and verify persisted account and message state");
    let process = configuration.start(&client)?;
    check_startup_summary(
        &configuration.stdout()?,
        &configuration.database_path(),
        configuration.web_port,
        configuration.mail_port,
        configuration.imap_port,
        2,
    );
    check_logs(&configuration, &[&smtp_password, &imap_password])?;
    assert_eq!(
        client.get_json("/api/v1/instance")?,
        json!({"initialized": true, "setupKeyRequired": false})
    );
    assert_eq!(
        configuration.stdout()?.matches("Bootstrap secret:").count(),
        1
    );
    client.session_token = Some(text(
        &client.post_json("/api/v1/login", credentials)?,
        "token",
    )?);
    assert!(
        client
            .get_json("/api/v1/messages")?
            .as_array()
            .ok_or("messages must be an array")?
            .iter()
            .any(|message| message["id"] == message_identifier)
    );
    mail::send_mail(
        configuration.mail_port,
        "",
        "bounce@recipient.test",
        b"Subject: null sender\r\n\r\nbounce\r\n",
        None,
    )?;
    wait_for_messages(&client, 3)?;
    mail::send_mail(
        configuration.mail_port,
        "app@sender.test",
        "user@recipient.test",
        b"Subject: authenticated\r\n\r\nvia SMTP access\r\n",
        Some((&smtp_username, &smtp_password)),
    )?;
    wait_for_messages(&client, 4)?;
    mail::check_imap_restart(
        configuration.imap_port,
        &imap_username,
        &imap_password,
        &validity,
    )?;
    process.stop()?;

    progress.start("Rebuild search offline and verify persisted SQLite data");
    let (status, output, errors) = configuration.complete(
        configuration.command().args(["search", "rebuild"]),
        Duration::from_secs(30),
    )?;
    assert!(
        status.success() && output.contains("pending=0"),
        "{output}{errors}"
    );
    assert!(!output.contains("Bootstrap secret:"));
    check_persisted_database(
        &configuration.database_path(),
        &message_identifier,
        &captured_message,
    )
    .await?;
    #[cfg(unix)]
    assert!(configuration.stderr()?.contains("shutting down listeners"));
    assert!(!configuration.stdout()?.contains("shutting down listeners"));
    check_logs(&configuration, &[&smtp_password, &imap_password])?;

    progress.start("Verify startup bind failure");
    let occupied = TcpListener::bind(("127.0.0.1", 0))?;
    let mut command = configuration.command();
    command.env(
        "SANDPOST_SMTP_PORT",
        occupied.local_addr()?.port().to_string(),
    );
    let errors = check_failed_startup(&configuration, &mut command)?;
    assert!(
        errors.to_lowercase().contains("address already in use") || errors.contains("10048"),
        "{errors}"
    );
    drop(occupied);
    progress.start("Verify startup storage failure");
    let invalid_database = directory.path().join("not-a-database");
    fs::create_dir(&invalid_database)?;
    check_failed_startup(
        &configuration,
        configuration
            .command()
            .env("SANDPOST_DATABASE_PATH", invalid_database),
    )?;
    progress.start("Verify port-zero listener allocation and connectivity");
    check_port_zero(&configuration)?;
    progress.finish();
    println!(
        "PASS: startup summary, bind/storage failures, port 0, SMTP, normalization, API, raw MIME, SSE, search status/rebuild/locking, IMAP (IDLE, MOVE, UID persistence), persistence, restart, shutdown{}",
        if frontend.is_some() {
            ", static assets and SPA routing"
        } else {
            ""
        }
    );
    Ok(())
}

/// Require a nonempty string in a JSON response rather than accepting missing credentials.
fn text(value: &Value, field: &str) -> Result<String> {
    value[field]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("missing string field: {field}").into())
}

/// Poll the eventual search index until the expected committed message count is visible.
fn wait_for_messages(client: &HttpClient, expected: usize) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let messages = client.get_json("/api/v1/messages")?;
        if messages
            .as_array()
            .is_some_and(|messages| messages.len() == expected)
        {
            return Ok(messages);
        }
        if Instant::now() >= deadline {
            return Err(
                format!("index did not converge to {expected} messages: {messages}").into(),
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
}

/// Verify secrets stay out of listing endpoints, which must expose only credential metadata.
fn check_listed_credentials(listing: &Value, username: &str, password: &str) {
    let listing = listing.to_string();
    assert!(listing.contains(username));
    assert!(!listing.contains(password));
    assert!(!listing.contains("password"));
}

/// Verify stdout summaries and stderr tracing remain plain text and never expose passwords.
fn check_logs(configuration: &ServerConfiguration, passwords: &[&str]) -> Result<()> {
    let logs = configuration.logs()?;
    assert!(!logs.contains('\u{1b}'));
    assert!(!logs.contains(SECRET_SENTINEL));
    for password in passwords {
        assert!(!logs.contains(password));
    }
    assert!(!configuration.stderr()?.contains("Ready to receive mail."));
    Ok(())
}

/// Check the count and configured listener values of every successful startup summary.
fn check_startup_summary(
    output: &str,
    database: &Path,
    web_port: u16,
    mail_port: u16,
    imap_port: u16,
    count: usize,
) {
    let rows: Vec<String> = output
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(
        rows.iter()
            .filter(|row| row.as_str() == "Ready to receive mail.")
            .count(),
        count,
        "{output}"
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row.as_str() == "Host 127.0.0.1")
            .count(),
        count * 2,
        "{output}"
    );
    for expected in [
        "Sandpost".into(),
        format!("Port {mail_port}"),
        format!("Port {imap_port}"),
        "Security none (passwords in clear text)".into(),
        format!("Listening 127.0.0.1:{imap_port}"),
        "Encryption none".into(),
        "Auth optional (unauthenticated local mail accepted)".into(),
        format!("Listening 127.0.0.1:{mail_port}"),
        format!("API http://127.0.0.1:{web_port}/api/v1/messages"),
        format!("Listening 127.0.0.1:{web_port}"),
        format!("Database SQLite · {}", database.display()),
        "Attachments in original messages (SQLite)".into(),
    ] {
        assert_eq!(
            rows.iter().filter(|row| *row == &expected).count(),
            count,
            "missing summary value {expected}: {output}"
        );
    }
}

/// Compare embedded frontend bytes, deep SPA routes, assets and API 404 behavior.
fn check_frontend(client: &HttpClient, directory: &Path) -> Result<()> {
    let index = fs::read(directory.join("index.html"))?;
    for path in ["/", "/mail", "/mail/00000000-0000-4000-8000-000000000001"] {
        let response = client.request("GET", path, None)?;
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "text/html");
        assert_eq!(response.body, index);
    }
    let source = std::str::from_utf8(&index)?;
    let assets: Vec<&str> = source
        .split(['<', '>'])
        .flat_map(|tag| tag.split_whitespace())
        .filter_map(|attribute| {
            attribute
                .strip_prefix("src=\"/assets/")
                .or_else(|| attribute.strip_prefix("href=\"/assets/"))
        })
        .filter_map(|value| value.split_once('"').map(|(path, _)| path))
        .collect();
    assert!(
        !assets.is_empty(),
        "frontend index must reference built assets"
    );
    for asset in assets {
        let response = client.request("GET", &format!("/assets/{asset}"), None)?;
        assert_eq!(response.status, 200);
        assert_eq!(
            response.body,
            fs::read(directory.join("assets").join(asset))?
        );
    }
    let missing = client.request("GET", "/api/v1/not-implemented", None)?;
    assert_eq!(missing.status, 404);
    assert_ne!(missing.body, index);
    Ok(())
}

/// Open an existing database without creating files or changing the server's state.
async fn database(path: &Path) -> Result<SqliteConnection> {
    Ok(
        SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(path).read_only(true))
            .await?,
    )
}

/// Check committed mail, WAL mode, removed legacy schema, and exact migration history.
async fn check_ingested_database(path: &Path) -> Result<()> {
    let mut connection = database(path).await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("select count(*) from mail")
            .fetch_one(&mut connection)
            .await?,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "select count(*) from sqlite_schema where name in ('mail_scope', 'endpoints')"
        )
        .fetch_one(&mut connection)
        .await?,
        0
    );
    let migrations =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../sandpost-storage-sqlite/migrations");
    let mut expected = fs::read_dir(migrations)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    expected.sort();
    assert!(!expected.is_empty());
    assert_eq!(
        sqlx::query_scalar::<_, String>("select migration from migrations order by migration")
            .fetch_all(&mut connection)
            .await?,
        expected
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("pragma journal_mode")
            .fetch_one(&mut connection)
            .await?,
        "wal"
    );
    connection.close().await?;
    Ok(())
}

/// Verify persisted credential material is an Argon2id hash and excludes the issued password.
async fn check_password_hash(path: &Path, query: &'static str, password: &str) -> Result<()> {
    let mut connection = database(path).await?;
    let hash: String = sqlx::query_scalar(query).fetch_one(&mut connection).await?;
    assert!(hash.starts_with("$argon2id$"));
    assert!(!hash.contains(password));
    connection.close().await?;
    Ok(())
}

/// Verify raw message bytes and credential use timestamps survive restart and offline rebuild.
async fn check_persisted_database(
    path: &Path,
    identifier: &str,
    captured_message: &[u8],
) -> Result<()> {
    let mut connection = database(path).await?;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("select count(*) from mail")
            .fetch_one(&mut connection)
            .await?,
        4
    );
    for query in [
        "select count(*) from imap_accounts where last_used_at is not null",
        "select count(*) from smtp_accesses where last_used_at is not null",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(query)
                .fetch_one(&mut connection)
                .await?,
            1
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, Vec<u8>>("select raw_message from mail where identifier=?")
            .bind(identifier)
            .fetch_one(&mut connection)
            .await?,
        captured_message
    );
    connection.close().await?;
    Ok(())
}

/// Require a startup error to exit unsuccessfully without readiness, secrets or terminal escapes.
fn check_failed_startup(
    configuration: &ServerConfiguration,
    command: &mut std::process::Command,
) -> Result<String> {
    let (status, output, errors) = configuration.complete(command, Duration::from_secs(10))?;
    assert!(!status.success(), "startup unexpectedly succeeded");
    assert!(!output.contains("Ready to receive mail."));
    assert!(!format!("{output}{errors}").contains(SECRET_SENTINEL));
    assert!(!format!("{output}{errors}").contains('\u{1b}'));
    Ok(errors)
}

/// Verify port zero reports actual reachable HTTP, SMTP and IMAP listeners even with tracing off.
fn check_port_zero(configuration: &ServerConfiguration) -> Result<()> {
    let directory = configuration.directory.join("port-zero");
    fs::create_dir(&directory)?;
    let database = directory.join("sandpost.sqlite3");
    let offset = configuration.stdout()?.len();
    let mut command = configuration.command();
    command
        .env("SANDPOST_DATA_DIRECTORY", &directory)
        .env("SANDPOST_DATABASE_PATH", &database)
        .env("SANDPOST_HTTP_PORT", "0")
        .env("SANDPOST_SMTP_PORT", "0")
        .env("SANDPOST_IMAP_PORT", "0")
        .env("SANDPOST_LOG_LEVEL", "off");
    let mut process = configuration.spawn(&mut command)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let output = loop {
        process.require_running()?;
        let output = configuration.stdout()?[offset..].to_owned();
        if output.contains("Ready to receive mail.") {
            break output;
        }
        if Instant::now() >= deadline {
            return Err(format!("port zero readiness timeout: {output}").into());
        }
        thread::sleep(Duration::from_millis(50));
    };
    let ports = output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Listening"))
        .filter_map(|line| line.trim().strip_prefix("127.0.0.1:"))
        .map(str::parse::<u16>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(ports.len(), 3, "{output}");
    assert!(ports.iter().all(|port| *port != 0));
    check_startup_summary(&output, &database, ports[2], ports[0], ports[1], 1);
    let client = HttpClient {
        port: ports[2],
        session_token: None,
    };
    assert_eq!(client.get_json("/api/v1/health")?["status"], "ok");
    for (port, prefix) in [
        (ports[0], "220 "),
        (ports[1], "* OK [CAPABILITY IMAP4rev1 "),
    ] {
        let stream =
            TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(3))?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        let mut greeting = String::new();
        BufReader::new(stream).read_line(&mut greeting)?;
        assert!(greeting.starts_with(prefix), "{greeting}");
    }
    check_logs(configuration, &[])?;
    process.stop()?;
    Ok(())
}
