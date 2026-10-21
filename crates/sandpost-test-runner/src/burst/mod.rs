//! Synthetic traffic sent to an existing SMTP server, in one full burst or repeated patterns.
mod pattern;

use crate::{Result, smtp::SmtpClient};
use pattern::{Burst, Quantity, parse_count, parse_duration, parse_pattern};
use std::{
    ffi::OsString,
    io::IsTerminal,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const USAGE: &str = "Usage:\n  sandpost-test-runner mail-burst full --count 50k [options]\n  sandpost-test-runner mail-burst tiny --pattern '50,10,1,50' [options]\n\nModes:\n  full   Send --count messages consecutively, then exit\n  tiny   Send --pattern bursts; --cycles defaults to 1, 0 repeats until Ctrl+C\n\nCommon options:\n  --host HOST             SMTP host (default: 127.0.0.1)\n  --port PORT             SMTP port (default: 1025)\n  --format FORMAT         txt (default), html or newsletter; HTML includes a text alternative\n  --to ADDRESS            Recipient (default: developer@sandpost.test)\n  --username USER         Authenticate with SMTP AUTH LOGIN\n  --password-env NAME     Password environment variable (default: SANDPOST_SMTP_PASSWORD)\n\nTiny options:\n  --interval DURATION     Pause between bursts and cycles (default: 1s)\n  --cycles COUNT          Pattern repetitions (default: 1; 0 means continuous)\n\nPatterns use positive counts, optional k suffix, rand(min,max), and pauses in ms/s/m.\nExample: --pattern '50,10s,10,1s,rand(10,100)'\nExplicit pauses replace --interval. No pause follows the final finite burst.\nUses one connection per burst; no automatic retries. Ctrl+C prints a summary and exits 130.";

struct Options {
    host: String,
    port: u16,
    recipient: String,
    message_format: MessageFormat,
    username: Option<String>,
    password: Option<String>,
    bursts: Vec<Burst>,
    cycles: u64,
    expected_total: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MessageFormat {
    Plain,
    Html,
    Newsletter,
}

struct Statistics {
    accepted: u64,
    cycle: u64,
    group: usize,
    group_accepted: u64,
    group_target: u64,
    started: Instant,
    last_report: Instant,
}

/// Parse traffic options, register interruption handling, and preserve totals on success or failure.
pub async fn run(arguments: &[OsString]) -> Result<ExitCode> {
    let Some(options) = parse_arguments(arguments)? else {
        return Ok(ExitCode::SUCCESS);
    };
    let (interrupted, signal_task) = interruption_handler()?;
    let mut statistics = Statistics::new();
    let result = execute(&options, &interrupted, &mut statistics);
    signal_task.abort();
    match result {
        Ok(true) => {
            report("STOP", &statistics.summary(options.expected_total));
            Ok(ExitCode::from(130))
        }
        Ok(false) => {
            report("PASS", &statistics.summary(options.expected_total));
            Ok(ExitCode::SUCCESS)
        }
        Err(error) if interrupted.load(Ordering::Relaxed) => {
            report("STOP", &statistics.summary(options.expected_total));
            report(
                "STOP",
                &format!(
                    "interrupted SMTP operation: {error}; only confirmed acceptances are counted, no retries"
                ),
            );
            Ok(ExitCode::from(130))
        }
        Err(error) => {
            report("FAIL", &statistics.summary(options.expected_total));
            Err(format!(
                "cycle {}, burst {}: {error}; no messages were retried",
                statistics.cycle, statistics.group
            )
            .into())
        }
    }
}

/**
 * Validate all command arguments and bounds before opening any network connection.
 */
fn parse_arguments(arguments: &[OsString]) -> Result<Option<Options>> {
    if arguments.is_empty()
        || arguments
            .iter()
            .any(|argument| argument == "--help" || argument == "-h")
    {
        println!("{USAGE}");
        return if arguments.is_empty() {
            Err("mail-burst requires full or tiny".into())
        } else {
            Ok(None)
        };
    }
    let mode = arguments[0].to_str().ok_or("mode must be UTF-8")?;
    if !matches!(mode, "full" | "tiny") {
        return Err(format!("unknown mail-burst mode: {mode}\n{USAGE}").into());
    }
    let mut values = std::collections::BTreeMap::new();
    let mut remaining = arguments[1..].iter();
    while let Some(argument) = remaining.next() {
        let option = argument.to_str().ok_or("option must be UTF-8")?;
        if !matches!(
            option,
            "--count"
                | "--pattern"
                | "--interval"
                | "--cycles"
                | "--host"
                | "--port"
                | "--format"
                | "--to"
                | "--username"
                | "--password-env"
        ) {
            return Err(format!("unknown option: {option}").into());
        }
        let value = remaining
            .next()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("{option} requires a UTF-8 value"))?;
        if values.insert(option, value).is_some() {
            return Err(format!("duplicate option: {option}").into());
        }
    }
    let host = values
        .get("--host")
        .copied()
        .unwrap_or("127.0.0.1")
        .to_owned();
    if host.trim().is_empty() || host.contains(['\r', '\n', '\0']) {
        return Err("invalid SMTP host".into());
    }
    let port = values
        .get("--port")
        .copied()
        .unwrap_or("1025")
        .parse::<u16>()?;
    if port == 0 {
        return Err("--port must be between 1 and 65535".into());
    }
    let recipient = values
        .get("--to")
        .copied()
        .unwrap_or("developer@sandpost.test");
    if recipient.trim() != recipient || sandpost_core::normalize_inbox_address(recipient).is_none()
    {
        return Err("--to must be a single ASCII email address without a display name".into());
    }
    let message_format = match values.get("--format").copied().unwrap_or("txt") {
        "txt" => MessageFormat::Plain,
        "html" => MessageFormat::Html,
        "newsletter" => MessageFormat::Newsletter,
        _ => return Err("--format must be txt, html or newsletter".into()),
    };
    let (bursts, cycles) = if mode == "full" {
        for option in ["--pattern", "--interval", "--cycles"] {
            if values.contains_key(option) {
                return Err(format!("{option} is only supported in tiny mode").into());
            }
        }
        let count = parse_count(
            values
                .get("--count")
                .copied()
                .ok_or("full requires --count")?,
        )?;
        (
            vec![Burst {
                quantity: Quantity::Fixed(count),
                delay: Duration::ZERO,
            }],
            1,
        )
    } else {
        if values.contains_key("--count") {
            return Err("--count is only supported in full mode".into());
        }
        let interval = parse_duration(values.get("--interval").copied().unwrap_or("1s"))?;
        let bursts = parse_pattern(
            values
                .get("--pattern")
                .copied()
                .ok_or("tiny requires --pattern")?,
            interval,
        )?;
        let cycles_text = values.get("--cycles").copied().unwrap_or("1");
        if cycles_text.is_empty() || !cycles_text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("--cycles must be a nonnegative integer".into());
        }
        (bursts, cycles_text.parse::<u64>()?)
    };
    let expected_total = finite_total(&bursts, cycles)?;
    let username = values.get("--username").map(|value| (*value).to_owned());
    if username
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.contains(['\r', '\n', '\0']))
    {
        return Err("invalid SMTP username".into());
    }
    let password = if username.is_some() {
        let variable = values
            .get("--password-env")
            .copied()
            .unwrap_or("SANDPOST_SMTP_PASSWORD");
        if variable.is_empty() || variable.contains(['=', '\0']) {
            return Err("invalid --password-env name".into());
        }
        Some(
            std::env::var(variable)
                .ok()
                .filter(|value| !value.is_empty() && !value.contains(['\r', '\n', '\0']))
                .ok_or_else(|| format!("set {variable} to a valid SMTP access password"))?,
        )
    } else {
        None
    };
    Ok(Some(Options {
        host,
        port,
        recipient: recipient.to_owned(),
        message_format,
        username,
        password,
        bursts,
        cycles,
        expected_total,
    }))
}

/// Reject finite counter overflows and return an exact total only for fixed finite patterns.
fn finite_total(bursts: &[Burst], cycles: u64) -> Result<Option<u64>> {
    if cycles == 0 {
        return Ok(None);
    }
    let mut maximum_total = 0_u64;
    let mut all_fixed = true;
    for burst in bursts {
        let maximum = match burst.quantity {
            Quantity::Fixed(count) => count,
            Quantity::Random { maximum, .. } => {
                all_fixed = false;
                maximum
            }
        };
        maximum_total = maximum_total
            .checked_add(maximum)
            .ok_or("pattern count overflows u64")?;
    }
    let maximum_total = maximum_total
        .checked_mul(cycles)
        .ok_or("total count overflows u64")?;
    Ok(all_fixed.then_some(maximum_total))
}

/**
 * Send each burst over one session, counting only final SMTP acceptance and never retrying sends.
 */
fn execute(
    options: &Options,
    interrupted: &AtomicBool,
    statistics: &mut Statistics,
) -> Result<bool> {
    let credentials = options.username.as_deref().zip(options.password.as_deref());
    let mut cycle = 0_u64;
    while options.cycles == 0 || cycle < options.cycles {
        if interrupted.load(Ordering::Relaxed) {
            return Ok(true);
        }
        cycle = cycle.checked_add(1).ok_or("cycle counter exhausted")?;
        statistics.cycle = cycle;
        for (group, burst) in options.bursts.iter().enumerate() {
            if interrupted.load(Ordering::Relaxed) {
                return Ok(true);
            }
            let quantity = sample_quantity(&burst.quantity);
            statistics.group = group + 1;
            statistics.group_target = quantity;
            statistics.group_accepted = 0;
            report(
                "RUN ",
                &format!(
                    "cycle {cycle}, burst {}/{}: send {quantity} emails to {}:{}",
                    group + 1,
                    options.bursts.len(),
                    options.host,
                    options.port
                ),
            );
            let mut connection = SmtpClient::connect(&options.host, options.port, credentials)?;
            for number in 1..=quantity {
                if interrupted.load(Ordering::Relaxed) {
                    return Ok(true);
                }
                let next_accepted = statistics
                    .accepted
                    .checked_add(1)
                    .ok_or("accepted counter exhausted")?;
                let message = make_message(
                    cycle,
                    group + 1,
                    number,
                    options.message_format,
                    &options.recipient,
                )?;
                connection.send("mail-burst@sandpost.test", &options.recipient, &message)?;
                statistics.accepted = next_accepted;
                statistics.group_accepted = number;
                if statistics.last_report.elapsed() >= Duration::from_millis(500) {
                    report("SEND", &statistics.summary(options.expected_total));
                    statistics.last_report = Instant::now();
                }
            }
            connection.quit()?;
            report("PASS", &statistics.summary(options.expected_total));
            let final_burst =
                options.cycles != 0 && cycle == options.cycles && group + 1 == options.bursts.len();
            if !final_burst && !burst.delay.is_zero() {
                report(
                    "WAIT",
                    &format!("{:.3}s before next burst", burst.delay.as_secs_f64()),
                );
                if wait_between_bursts(burst.delay, interrupted) {
                    return Ok(true);
                }
            }
        }
    }
    Ok(interrupted.load(Ordering::Relaxed))
}

/// Draw a fresh uniformly distributed inclusive random count using UUID random bytes.
fn sample_quantity(quantity: &Quantity) -> u64 {
    let (minimum, maximum) = match *quantity {
        Quantity::Fixed(count) => return count,
        Quantity::Random { minimum, maximum } => (minimum, maximum),
    };
    let width = maximum - minimum + 1;
    let rejection_threshold = width.wrapping_neg() % width;
    loop {
        let identifier = Uuid::new_v4();
        let bytes = identifier.as_bytes();
        // These eight bytes exclude the UUID version and variant bits.
        let sample = u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[10], bytes[11],
        ]);
        if sample >= rejection_threshold {
            return minimum + sample % width;
        }
    }
}

/**
 * Generate one unique plain-text or multipart HTML message without retaining the burst in memory.
 * Body templates are embedded at build time and rendered with CRLF lines for 7bit transport.
 */
fn make_message(
    cycle: u64,
    group: usize,
    number: u64,
    message_format: MessageFormat,
    recipient: &str,
) -> Result<Vec<u8>> {
    let timestamp = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())?;
    let date = mail_parser::DateTime::from_timestamp(timestamp).to_rfc822();
    let identifier = Uuid::new_v4();
    let subject = match message_format {
        MessageFormat::Newsletter => {
            "Sandpost Weekly | A little more care, a better inbox".to_owned()
        }
        _ => format!("Sandpost | cycle {cycle} | burst {group} | email {number}"),
    };
    let headers = format!(
        "From: mail-burst@sandpost.test\r\nTo: {recipient}\r\nSubject: {subject}\r\nDate: {date}\r\nMessage-ID: <{identifier}@example.test>\r\nMIME-Version: 1.0\r\n"
    );
    let plain_headers =
        "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\n";
    let text_template = match message_format {
        MessageFormat::Plain => include_str!("../../templates/plain.txt"),
        MessageFormat::Html => include_str!("../../templates/delivery-report.txt"),
        MessageFormat::Newsletter => include_str!("../../templates/newsletter.txt"),
    };
    let text = render_template(text_template, cycle, group, number, &identifier);
    match message_format {
        MessageFormat::Plain => Ok(format!("{headers}{plain_headers}{text}").into_bytes()),
        MessageFormat::Html | MessageFormat::Newsletter => {
            let boundary = format!("sandpost-alternative-{identifier}");
            let html_template = match message_format {
                MessageFormat::Newsletter => include_str!("../../templates/newsletter.html"),
                _ => include_str!("../../templates/delivery-report.html"),
            };
            let html = render_template(html_template, cycle, group, number, &identifier);
            Ok(format!("{headers}Content-Type: multipart/alternative;\r\n boundary=\"{boundary}\"\r\n\r\n--{boundary}\r\n{plain_headers}{text}\r\n--{boundary}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\n{html}\r\n--{boundary}--\r\n").into_bytes())
        }
    }
}

/**
 * Substitute numeric burst metadata and the message identifier, normalizing template lines to CRLF.
 */
fn render_template(
    template: &str,
    cycle: u64,
    group: usize,
    number: u64,
    identifier: &Uuid,
) -> String {
    template
        .replace("{{cycle}}", &cycle.to_string())
        .replace("{{group}}", &group.to_string())
        .replace("{{number}}", &number.to_string())
        .replace("{{identifier}}", &identifier.to_string())
        .replace("\r\n", "\n")
        .replace('\n', "\r\n")
}

/// Wait after an accepted burst, allowing Ctrl+C to interrupt long pauses promptly.
fn wait_between_bursts(duration: Duration, interrupted: &AtomicBool) -> bool {
    let started = Instant::now();
    loop {
        if interrupted.load(Ordering::Relaxed) {
            return true;
        }
        let remaining = duration.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return false;
        }
        thread::sleep(remaining.min(Duration::from_millis(50)));
    }
}

/// Install the platform's Ctrl+C listener before starting synchronous SMTP operations.
fn interruption_handler() -> Result<(Arc<AtomicBool>, tokio::task::JoinHandle<()>)> {
    let interrupted = Arc::new(AtomicBool::new(false));
    let signal_flag = Arc::clone(&interrupted);
    #[cfg(unix)]
    let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    #[cfg(windows)]
    let mut signal = tokio::signal::windows::ctrl_c()?;
    let task = tokio::spawn(async move {
        if signal.recv().await.is_some() {
            signal_flag.store(true, Ordering::Relaxed);
        }
    });
    Ok((interrupted, task))
}

/// Print readable append-only traffic status, honoring the same terminal color rules as smoke checks.
fn report(status: &str, message: &str) {
    let color = std::io::stderr().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var_os("TERM").is_none_or(|value| value != "dumb");
    let code = match status {
        "PASS" => "32",
        "FAIL" => "31",
        _ => "33",
    };
    if color {
        eprintln!("\x1b[{code}m[{status}]\x1b[0m {message}");
    } else {
        eprintln!("[{status}] {message}");
    }
}

impl Statistics {
    /// Initialize counters before the first connection so connection errors still produce a summary.
    fn new() -> Self {
        let now = Instant::now();
        Self {
            accepted: 0,
            cycle: 0,
            group: 0,
            group_accepted: 0,
            group_target: 0,
            started: now,
            last_report: now,
        }
    }

    /// Format acknowledged group progress, aggregate acceptance, duration and observed throughput.
    fn summary(&self, expected_total: Option<u64>) -> String {
        let seconds = self.started.elapsed().as_secs_f64();
        let total = expected_total.map_or_else(
            || self.accepted.to_string(),
            |total| format!("{}/{total}", self.accepted),
        );
        let rate = self.accepted as f64 / seconds.max(0.001);
        format!(
            "cycle {}, burst {}: accepted {}/{}, total {total}, elapsed {seconds:.3}s, {rate:.1} emails/s",
            self.cycle, self.group, self.group_accepted, self.group_target
        )
    }
}

#[cfg(test)]
mod tests;
