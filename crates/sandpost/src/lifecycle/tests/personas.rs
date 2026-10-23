//! A Persona reacting to mail captured by the real SMTP listener.

use super::*;
use sandpost_application::{PersonaRequest, PersonaSettings};
use sandpost_core::PersonaExecutionStatus;

/**
 * Start a loopback HTTP server that records request paths and answers 200.
 */
async fn recording_server() -> (SocketAddr, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let paths = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = paths.clone();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let recorded = recorded.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buffer = [0_u8; 1024];
                while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => head.extend_from_slice(&buffer[..read]),
                    }
                }
                let path = String::from_utf8_lossy(&head)
                    .split(' ')
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                recorded.lock().unwrap().push(path);
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
            });
        }
    });
    (address, paths)
}

/**
 * Submit one HTML message over plain SMTP to the given envelope recipient.
 */
pub(super) async fn submit_html(address: SocketAddr, recipient: &str, subject: &str, html: &str) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut stream = BufReader::new(tokio::net::TcpStream::connect(address).await.unwrap());
    let mut line = String::new();
    stream.read_line(&mut line).await.unwrap();
    for command in [
        "EHLO example.test\r\n".to_owned(),
        "MAIL FROM:<app@example.test>\r\n".to_owned(),
        format!("RCPT TO:<{recipient}>\r\n"),
        "DATA\r\n".to_owned(),
        format!(
            "Subject: {subject}\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n.\r\n"
        ),
        "QUIT\r\n".to_owned(),
    ] {
        stream
            .get_mut()
            .write_all(command.as_bytes())
            .await
            .unwrap();
        loop {
            let mut reply = String::new();
            if stream.read_line(&mut reply).await.unwrap() == 0 {
                return;
            }
            assert!(
                !reply.starts_with('4') && !reply.starts_with('5'),
                "{reply}"
            );
            if reply.as_bytes().get(3) != Some(&b'-') {
                break;
            }
        }
    }
}

/// SMTP capture schedules the Persona durably and the worker opens and clicks asynchronously.
#[tokio::test]
async fn captured_mail_triggers_personas_end_to_end() {
    let (mut configuration, _directory) = test_configuration();
    configuration.personas.allow_internal_addresses = true;
    let running = SandpostRuntime::start(configuration).await.unwrap();
    assert_eq!(
        running.application.persona_settings(),
        &PersonaSettings {
            allow_internal_addresses: true,
            ..PersonaSettings::default()
        }
    );
    let instance = running.application.instance();
    let secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
    instance
        .setup(sandpost_application::SetupRequest {
            bootstrap_secret: secret,
            name: "Owner".into(),
            email: "owner@example.test".into(),
            password: "runtime test password".into(),
        })
        .await
        .unwrap();
    let login = instance
        .login(sandpost_application::LoginRequest {
            remember_me: false,
            email: "owner@example.test".into(),
            password: "runtime test password".into(),
        })
        .await
        .unwrap();
    let owner = running.application.authorized(&login.user);
    let persona = owner
        .create_persona(PersonaRequest {
            name: "Alice".into(),
            address: "alice@example.test".into(),
            icon: None,
            script: "open; if (subject contains \"Verify\") { click link(text: \"Verify*\"); }"
                .into(),
            enabled: true,
        })
        .await
        .unwrap()
        .persona
        .identifier;
    let (server, paths) = recording_server().await;
    submit_html(
        running.mail_listen_address,
        "Alice@example.test",
        "Verify your account",
        &format!(
            "<img src=\"http://{server}/open.gif\"><a href=\"http://{server}/verify?t=1\">Verify my account</a>"
        ),
    )
    .await;
    let mut status = None;
    for _ in 0..300 {
        if let Some(summary) = owner
            .list_persona_executions(persona, None, 1)
            .await
            .unwrap()
            .into_iter()
            .next()
            && summary.execution.status.is_terminal()
        {
            status = Some(summary.execution.status);
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(status, Some(PersonaExecutionStatus::Completed));
    assert_eq!(*paths.lock().unwrap(), ["/open.gif", "/verify?t=1"]);
    running.shutdown().await.unwrap();
}
