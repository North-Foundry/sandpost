//! Isolated installations and wire clients shared by IMAP runtime tests.

use crate::{Configuration, RunningSandpost};
use sandpost_application::{IssuedImapCredentials, LoginRequest, SessionUser, SetupRequest};
use std::{net::SocketAddr, path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    time::timeout,
};

/// Own isolated runtime data and remove it when the test ends.
pub(super) struct TestDirectory(PathBuf);

impl Drop for TestDirectory {
    /// Remove the isolated installation.
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build configuration with system-allocated ports for HTTP, SMTP, and IMAP.
pub(super) fn configuration() -> (Configuration, TestDirectory) {
    let directory =
        TestDirectory(std::env::temp_dir().join(format!("sandpost-imap-{}", uuid::Uuid::new_v4())));
    let configuration = Configuration::from_values(|name| match name {
        "SANDPOST_DATA_DIRECTORY" => Some(directory.0.to_string_lossy().into_owned()),
        "SANDPOST_HTTP_LISTEN" | "SANDPOST_SMTP_LISTEN" | "SANDPOST_IMAP_LISTEN" => {
            Some("127.0.0.1:0".into())
        }
        _ => None,
    })
    .unwrap();
    (configuration, directory)
}

/// Complete setup and return the Owner's session identity.
pub(super) async fn owner(running: &RunningSandpost) -> SessionUser {
    let instance = running.application.instance();
    if let Some(secret) = instance.bootstrap_secret_for_startup().unwrap() {
        instance
            .setup(SetupRequest {
                bootstrap_secret: secret,
                name: "Owner".into(),
                email: "owner@example.test".into(),
                password: "runtime test password".into(),
            })
            .await
            .unwrap();
    }
    instance
        .login(LoginRequest {
            email: "owner@example.test".into(),
            password: "runtime test password".into(),
        })
        .await
        .unwrap()
        .user
}

/// The IMAP address of a running runtime.
pub(super) fn imap_address(running: &RunningSandpost) -> SocketAddr {
    running.imap.as_ref().unwrap().listen_address.unwrap()
}

/// Send one unauthenticated message over local SMTP.
pub(super) async fn send_mail(address: SocketAddr, recipient: &str, subject: &str) {
    let mut stream = BufReader::new(TcpStream::connect(address).await.unwrap());
    let mut line = String::new();
    stream.read_line(&mut line).await.unwrap();
    for command in [
        "EHLO example.test\r\n".to_owned(),
        "MAIL FROM:<app@example.test>\r\n".to_owned(),
        format!("RCPT TO:<{recipient}>\r\n"),
        "DATA\r\n".to_owned(),
        format!(
            "From: app@example.test\r\nTo: {recipient}\r\nSubject: {subject}\r\n\r\nHello.\r\n.\r\n"
        ),
        "QUIT\r\n".to_owned(),
    ] {
        stream
            .get_mut()
            .write_all(command.as_bytes())
            .await
            .unwrap();
        loop {
            line.clear();
            stream.read_line(&mut line).await.unwrap();
            if line.as_bytes().get(3) != Some(&b'-') {
                break;
            }
        }
        assert!(
            !line.starts_with('4') && !line.starts_with('5'),
            "{command}: {line}"
        );
    }
}

/// A raw IMAP client that reads complete responses including literals.
pub(super) struct Client(pub(super) BufReader<TcpStream>);

impl Client {
    /// Connect and read the greeting.
    pub(super) async fn connect(address: SocketAddr) -> Self {
        let mut client = Self(BufReader::new(TcpStream::connect(address).await.unwrap()));
        let greeting = client.response().await;
        assert!(greeting.starts_with("* OK"), "{greeting}");
        client
    }

    /// Read one response with its literals.
    pub(super) async fn response(&mut self) -> String {
        let mut response = Vec::new();
        loop {
            let mut line = Vec::new();
            timeout(Duration::from_secs(5), self.0.read_until(b'\n', &mut line))
                .await
                .expect("IMAP response timed out")
                .unwrap();
            assert!(!line.is_empty(), "connection closed");
            response.extend_from_slice(&line);
            let trimmed = line.strip_suffix(b"}\r\n");
            let Some(length) = trimmed.and_then(|body| {
                let open = body.iter().rposition(|byte| *byte == b'{')?;
                std::str::from_utf8(&body[open + 1..])
                    .ok()?
                    .parse::<usize>()
                    .ok()
            }) else {
                return String::from_utf8_lossy(&response).into_owned();
            };
            let mut literal = vec![0; length];
            self.0.read_exact(&mut literal).await.unwrap();
            response.extend_from_slice(&literal);
        }
    }

    /// Run a tagged command and return every response through its completion.
    pub(super) async fn command(&mut self, tag: &str, command: &str) -> Vec<String> {
        self.0
            .get_mut()
            .write_all(format!("{tag} {command}\r\n").as_bytes())
            .await
            .unwrap();
        let mut responses = Vec::new();
        loop {
            let response = self.response().await;
            let done = response.starts_with(&format!("{tag} "));
            responses.push(response);
            if done {
                return responses;
            }
        }
    }

    /// Log in with issued credentials.
    pub(super) async fn login(address: SocketAddr, issued: &IssuedImapCredentials) -> Self {
        let mut client = Self::connect(address).await;
        let completion = client
            .command(
                "l",
                &format!(
                    "LOGIN {} \"{}\"",
                    issued.account.account.username, issued.password
                ),
            )
            .await;
        assert!(
            completion.last().unwrap().starts_with("l OK"),
            "{completion:?}"
        );
        client
    }
}

/// Wait until the search index has caught up so SMTP mail is fully captured.
pub(super) async fn wait_for_mail(running: &RunningSandpost, count: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let status = running
            .application
            .search_coordinator()
            .status()
            .await
            .unwrap();
        if status.latest_sequence >= count {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "mail was not captured"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Read UIDVALIDITY from a SELECT or EXAMINE response.
pub(super) fn uid_validity(responses: &[String]) -> String {
    responses
        .iter()
        .find_map(|line| line.strip_prefix("* OK [UIDVALIDITY "))
        .and_then(|rest| rest.split(']').next())
        .expect("UIDVALIDITY response")
        .to_owned()
}

/// Send an authenticated HTTP request through the running server and retain its wire response.
pub(super) async fn http_request(
    running: &RunningSandpost,
    token: &str,
    method: &str,
    path: &str,
) -> String {
    let mut connection = TcpStream::connect(running.web_listen_address)
        .await
        .unwrap();
    connection.write_all(format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        running.web_listen_address,
    ).as_bytes()).await.unwrap();
    let mut response = Vec::new();
    timeout(
        Duration::from_secs(5),
        connection.read_to_end(&mut response),
    )
    .await
    .unwrap()
    .unwrap();
    String::from_utf8(response).unwrap()
}
