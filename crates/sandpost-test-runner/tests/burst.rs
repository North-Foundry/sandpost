use mail_parser::MimeHeaders;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

const ACCEPT_TIMEOUT: Duration = Duration::from_secs(5);
const SOCKET_TIMEOUT: Duration = Duration::from_secs(3);
const PROCESS_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LINE_BYTES: usize = 4_096;
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

struct SessionReport {
    ehlo_count: usize,
    senders: Vec<String>,
    recipients: Vec<String>,
    accepted_messages: Vec<Vec<u8>>,
}

struct ServerReport {
    sessions: Vec<SessionReport>,
    attempts: usize,
    rejected: usize,
    extra_connection: bool,
}

struct RunningCli {
    child: ChildGuard,
    _logs: tempfile::TempDir,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

struct ChildGuard(Child);

/**
 * Verify one full burst uses one SMTP session, sends three accepted messages, and emits distinct MIME identifiers.
 */
#[test]
fn full_burst_uses_one_session_and_unique_message_ids() {
    let (address, server) = start_server(1, None);
    let mut cli = spawn_cli(&[
        "mail-burst",
        "full",
        "--count",
        "3",
        "--host",
        "127.0.0.1",
        "--port",
        &address.port().to_string(),
    ]);
    let status = wait_for_exit(&mut cli, PROCESS_TIMEOUT);
    assert!(status.success(), "{}", cli.stderr());
    let report = join_server(server);
    assert_eq!(report.sessions.len(), 1);
    assert_eq!(report.sessions[0].ehlo_count, 1);
    assert_eq!(report.attempts, 3);
    assert_eq!(report.sessions[0].accepted_messages.len(), 3);
    assert_eq!(
        report.sessions[0].senders,
        vec!["<mail-burst@sandpost.test>"; 3]
    );
    assert_eq!(
        report.sessions[0].recipients,
        vec!["<developer@sandpost.test>"; 3]
    );
    let message_ids: Vec<_> = report.sessions[0]
        .accepted_messages
        .iter()
        .map(|message| header_value(message, "Message-ID"))
        .collect();
    assert!(message_ids.iter().all(Option::is_some));
    assert_eq!(
        message_ids
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3
    );
    for message in &report.sessions[0].accepted_messages {
        assert_eq!(
            header_value(message, "From").as_deref(),
            Some("mail-burst@sandpost.test")
        );
        assert_eq!(
            header_value(message, "To").as_deref(),
            Some("developer@sandpost.test")
        );
        assert_eq!(
            header_value(message, "Content-Type").as_deref(),
            Some("text/plain; charset=utf-8")
        );
        assert!(header_value(message, "MIME-Version").is_some());
        assert!(header_value(message, "Content-Type").is_some());
    }
    assert!(!report.extra_connection);
}

/**
 * Verify one full HTML burst delivers a parseable multipart alternative with both bodies and burst metadata.
 */
#[test]
fn full_burst_html_format_sends_multipart_alternative() {
    let (address, server) = start_server(1, None);
    let mut cli = spawn_cli(&[
        "mail-burst",
        "full",
        "--count",
        "1",
        "--format",
        "html",
        "--to",
        "Persona+preview@Sandpost.test",
        "--host",
        "127.0.0.1",
        "--port",
        &address.port().to_string(),
    ]);
    let status = wait_for_exit(&mut cli, PROCESS_TIMEOUT);
    assert!(status.success(), "{}", cli.stderr());

    let report = join_server(server);
    assert_eq!(report.sessions.len(), 1);
    assert_eq!(report.sessions[0].accepted_messages.len(), 1);
    assert_eq!(report.sessions[0].senders, ["<mail-burst@sandpost.test>"]);
    assert_eq!(
        report.sessions[0].recipients,
        ["<Persona+preview@Sandpost.test>"]
    );
    assert_eq!(
        header_value(&report.sessions[0].accepted_messages[0], "To").as_deref(),
        Some("Persona+preview@Sandpost.test")
    );
    assert_eq!(
        header_value(&report.sessions[0].accepted_messages[0], "From").as_deref(),
        Some("mail-burst@sandpost.test")
    );
    let message = mail_parser::MessageParser::default()
        .parse(&report.sessions[0].accepted_messages[0])
        .expect("parse accepted HTML message");
    assert!(message.message_id().is_some());
    assert!(message.is_content_type("multipart", "alternative"));

    let text = message.body_text(0).expect("plain-text alternative");
    assert!(text.contains("cycle 1, burst 1, email 1"), "{text}");
    let html = message.body_html(0).expect("HTML alternative");
    let html_lowercase = html.to_ascii_lowercase();
    assert!(html_lowercase.contains("<!doctype html>"), "{html}");
    assert!(html_lowercase.contains("<table"), "{html}");
    assert!(html_lowercase.contains("<style"), "{html}");
    assert!(html_lowercase.contains("<a "), "{html}");
    assert!(
        html.contains("cycle 1") && html.contains("burst 1") && html.contains("email 1"),
        "{html}"
    );
    assert!(!report.extra_connection);
}

/**
 * Verify two tiny cycles open one session per burst and receive exactly ten SMTP acceptances.
 */
#[test]
fn tiny_pattern_repeats_exact_counts_in_four_sessions() {
    let (address, server) = start_server(4, None);
    let mut cli = spawn_cli(&[
        "mail-burst",
        "tiny",
        "--pattern",
        "2,10ms,rand(3,3)",
        "--format",
        "txt",
        "--to",
        "persona@sandpost.test",
        "--cycles",
        "2",
        "--interval",
        "5ms",
        "--host",
        "127.0.0.1",
        "--port",
        &address.port().to_string(),
    ]);
    let started = Instant::now();
    let status = wait_for_exit(&mut cli, PROCESS_TIMEOUT);
    assert!(started.elapsed() >= Duration::from_millis(25));
    assert!(status.success(), "{}", cli.stderr());
    let report = join_server(server);
    assert_eq!(report.sessions.len(), 4);
    assert!(
        report
            .sessions
            .iter()
            .all(|session| session.ehlo_count == 1)
    );
    assert_eq!(
        report
            .sessions
            .iter()
            .map(|session| session.accepted_messages.len())
            .collect::<Vec<_>>(),
        [2, 3, 2, 3]
    );
    assert_eq!(report.attempts, 10);
    for session in &report.sessions {
        assert!(
            session
                .senders
                .iter()
                .all(|sender| sender == "<mail-burst@sandpost.test>")
        );
        assert!(
            session
                .recipients
                .iter()
                .all(|recipient| recipient == "<persona@sandpost.test>")
        );
        for message in &session.accepted_messages {
            assert_eq!(
                header_value(message, "To").as_deref(),
                Some("persona@sandpost.test")
            );
            assert_eq!(
                header_value(message, "Content-Type").as_deref(),
                Some("text/plain; charset=utf-8")
            );
        }
    }
    assert_eq!(report.rejected, 0);
    assert!(!report.extra_connection);
}

/// Verify a finite default cycle exits without waiting through its final thirty-second delay.
#[test]
fn finite_tiny_pattern_skips_final_delay() {
    let (address, server) = start_server(1, None);
    let mut cli = spawn_cli(&[
        "mail-burst",
        "tiny",
        "--pattern",
        "1,30s",
        "--host",
        "127.0.0.1",
        "--port",
        &address.port().to_string(),
    ]);
    let status = wait_for_exit(&mut cli, PROCESS_TIMEOUT);
    assert!(status.success(), "{}", cli.stderr());
    let report = join_server(server);
    assert_eq!(report.sessions.len(), 1);
    assert_eq!(report.attempts, 1);
    assert!(!report.extra_connection);
}

/// Verify a 451 on the third DATA causes failure after two acceptances without retry or reconnect.
#[test]
fn rejected_data_preserves_diagnostic_without_retry() {
    let (address, server) = start_server(1, Some(3));
    let mut cli = spawn_cli(&[
        "mail-burst",
        "full",
        "--count",
        "5",
        "--host",
        "127.0.0.1",
        "--port",
        &address.port().to_string(),
    ]);
    let status = wait_for_exit(&mut cli, PROCESS_TIMEOUT);
    assert!(!status.success());
    let output = cli.stderr();
    assert!(output.contains("accepted 2/5"), "{output}");
    assert!(output.contains("SMTP reply 451"), "{output}");
    assert!(output.contains("no messages were retried"), "{output}");
    let report = join_server(server);
    assert_eq!(report.sessions.len(), 1);
    assert_eq!(report.sessions[0].ehlo_count, 1);
    assert_eq!(report.attempts, 3);
    assert_eq!(report.sessions[0].accepted_messages.len(), 2);
    assert_eq!(report.rejected, 1);
    assert!(!report.extra_connection);
}

/// Verify SIGINT during a continuous thirty-second pause exits promptly with the accepted-count summary.
#[cfg(unix)]
#[test]
fn ctrl_c_stops_continuous_pattern_during_delay() {
    let (address, server) = start_server(1, None);
    let mut cli = spawn_cli(&[
        "mail-burst",
        "tiny",
        "--pattern",
        "1,30s",
        "--cycles",
        "0",
        "--host",
        "127.0.0.1",
        "--port",
        &address.port().to_string(),
    ]);
    let report = join_server(server);
    assert_eq!(report.attempts, 1);
    assert_eq!(report.sessions.len(), 1);
    assert!(!report.extra_connection);
    let process_id = cli.child.0.id() as libc::pid_t;
    // SAFETY: the child is owned by this test and remains alive until the signal is sent.
    let signal_result = unsafe { libc::kill(process_id, libc::SIGINT) };
    assert_eq!(signal_result, 0, "failed to send SIGINT to child");
    let status = wait_for_exit(&mut cli, Duration::from_secs(2));
    assert_eq!(status.code(), Some(130));
    let output = cli.stderr();
    assert!(output.contains("[STOP]"), "{output}");
    assert!(output.contains("accepted 1/"), "{output}");
}

/// Bind an ephemeral loopback listener and serve a fixed session count on a bounded worker thread.
fn start_server(
    expected_sessions: usize,
    reject_attempt: Option<usize>,
) -> (
    std::net::SocketAddr,
    thread::JoinHandle<Result<ServerReport, String>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake SMTP listener");
    listener
        .set_nonblocking(true)
        .expect("set listener nonblocking");
    let address = listener.local_addr().expect("read listener address");
    let server = thread::spawn(move || serve(listener, expected_sessions, reject_attempt));
    (address, server)
}

/// Accept expected SMTP sessions, apply an optional global DATA rejection, and watch briefly for retries.
fn serve(
    listener: TcpListener,
    expected_sessions: usize,
    reject_attempt: Option<usize>,
) -> Result<ServerReport, String> {
    let mut report = ServerReport {
        sessions: Vec::new(),
        attempts: 0,
        rejected: 0,
        extra_connection: false,
    };
    for _ in 0..expected_sessions {
        let mut stream = accept_bounded(&listener)?;
        stream
            .set_nonblocking(false)
            .map_err(|error| error.to_string())?;
        stream
            .set_read_timeout(Some(SOCKET_TIMEOUT))
            .map_err(|error| error.to_string())?;
        stream
            .set_write_timeout(Some(SOCKET_TIMEOUT))
            .map_err(|error| error.to_string())?;
        let session = serve_session(&mut stream, reject_attempt, &mut report)?;
        report.sessions.push(session);
    }
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _)) => {
                report.extra_connection = true;
                drop(stream);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(report)
}

/**
 * Serve one SMTP session, recording each completed DATA transaction and returning a configured 451.
 */
fn serve_session(
    stream: &mut TcpStream,
    reject_attempt: Option<usize>,
    report: &mut ServerReport,
) -> Result<SessionReport, String> {
    let mut reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
    write_reply(stream, "220 fake.smtp.test ready\r\n")?;
    let mut session = SessionReport {
        ehlo_count: 0,
        senders: Vec::new(),
        recipients: Vec::new(),
        accepted_messages: Vec::new(),
    };
    loop {
        let command = read_line(&mut reader, MAX_LINE_BYTES)?;
        match command.trim_end_matches(['\r', '\n']) {
            line if line.starts_with("EHLO ") => {
                session.ehlo_count += 1;
                write_reply(stream, "250-fake.smtp.test\r\n250 OK\r\n")?;
            }
            line if line.starts_with("MAIL FROM:") => {
                session.senders.push(line["MAIL FROM:".len()..].to_owned());
                write_reply(stream, "250 sender ok\r\n")?;
            }
            line if line.starts_with("RCPT TO:") => {
                session.recipients.push(line["RCPT TO:".len()..].to_owned());
                write_reply(stream, "250 recipient ok\r\n")?;
            }
            "DATA" => {
                write_reply(stream, "354 send message\r\n")?;
                let message = read_message(&mut reader)?;
                report.attempts += 1;
                if reject_attempt == Some(report.attempts) {
                    report.rejected += 1;
                    write_reply(stream, "451 4.3.0 deliberate test rejection\r\n")?;
                    return Ok(session);
                }
                session.accepted_messages.push(message);
                write_reply(stream, "250 message accepted\r\n")?;
            }
            "QUIT" => {
                write_reply(stream, "221 closing connection\r\n")?;
                return Ok(session);
            }
            unexpected => return Err(format!("unexpected SMTP command: {unexpected}")),
        }
    }
}

/// Accept one incoming socket while polling only until the configured deadline.
fn accept_bounded(listener: &TcpListener) -> Result<TcpStream, String> {
    let deadline = Instant::now() + ACCEPT_TIMEOUT;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("timed out waiting for SMTP connection".to_owned());
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

/// Read one CRLF-terminated protocol line and reject oversized input.
fn read_line(reader: &mut impl BufRead, maximum: usize) -> Result<String, String> {
    let mut line = Vec::new();
    let bytes = (&mut *reader)
        .take((maximum + 1) as u64)
        .read_until(b'\n', &mut line)
        .map_err(|error| error.to_string())?;
    if bytes == 0 {
        return Err("SMTP peer closed the connection".to_owned());
    }
    if bytes > maximum || !line.ends_with(b"\n") {
        return Err("SMTP line exceeded its bound or was incomplete".to_owned());
    }
    String::from_utf8(line).map_err(|error| error.to_string())
}

/// Read one bounded SMTP DATA payload through its dot terminator.
fn read_message(reader: &mut impl BufRead) -> Result<Vec<u8>, String> {
    let mut message = Vec::new();
    loop {
        let line = read_line(reader, MAX_LINE_BYTES)?;
        if line == ".\r\n" {
            return Ok(message);
        }
        message.extend_from_slice(line.as_bytes());
        if message.len() > MAX_MESSAGE_BYTES {
            return Err("SMTP message exceeded its bound".to_owned());
        }
    }
}

/// Write a complete fake-server reply under the socket's write timeout.
fn write_reply(stream: &mut TcpStream, reply: &str) -> Result<(), String> {
    stream
        .write_all(reply.as_bytes())
        .map_err(|error| error.to_string())
}

/// Launch the compiled runner with output captured in temporary log files.
fn spawn_cli(arguments: &[&str]) -> RunningCli {
    let logs = tempfile::tempdir().expect("create temporary CLI log directory");
    let stdout_path = logs.path().join("stdout.log");
    let stderr_path = logs.path().join("stderr.log");
    let stdout = File::create(&stdout_path).expect("create stdout log");
    let stderr = File::create(&stderr_path).expect("create stderr log");
    let child = Command::new(env!("CARGO_BIN_EXE_sandpost-test-runner"))
        .args(arguments)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn mail-burst CLI");
    RunningCli {
        child: ChildGuard(child),
        _logs: logs,
        stdout_path,
        stderr_path,
    }
}

/// Poll child completion until a deadline, killing and reaping it if it exceeds the bound.
fn wait_for_exit(cli: &mut RunningCli, timeout: Duration) -> ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = cli.child.0.try_wait().expect("poll child status") {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = cli.child.0.kill();
            let _ = cli.child.0.wait();
            panic!(
                "CLI exceeded {timeout:?}\nstdout:\n{}\nstderr:\n{}",
                cli.stdout(),
                cli.stderr()
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Read the captured standard error log after flushing it by observing child termination.
fn log_contents(path: &PathBuf) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| format!("<unable to read log: {error}>"))
}

/// Extract a case-insensitive MIME header value from the captured message bytes.
fn header_value(message: &[u8], name: &str) -> Option<String> {
    message
        .split(|byte| *byte == b'\n')
        .take_while(|line| !line.is_empty() && *line != b"\r")
        .filter_map(|line| std::str::from_utf8(line).ok())
        .find_map(|line| {
            let line = line.trim_end_matches('\r');
            let (header, value) = line.split_once(':')?;
            header
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
}

/// Join the bounded server worker and surface any protocol or socket failure.
fn join_server(server: thread::JoinHandle<Result<ServerReport, String>>) -> ServerReport {
    server
        .join()
        .expect("fake SMTP server worker panicked")
        .unwrap_or_else(|error| panic!("fake SMTP server failed: {error}"))
}

impl RunningCli {
    /// Read the child standard error log for assertion diagnostics.
    fn stderr(&self) -> String {
        log_contents(&self.stderr_path)
    }

    /// Read the child standard output log for timeout diagnostics.
    fn stdout(&self) -> String {
        log_contents(&self.stdout_path)
    }
}

impl Drop for ChildGuard {
    /// Kill and reap a still-running child so assertion failures cannot leak subprocesses.
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
