use super::*;
use mail_parser::MimeHeaders;

/// Build native CLI arguments without changing process environment during parallel tests.
fn arguments(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

/**
 * Verify full shorthand and default finite tiny patterns are resolved before network activity.
 */
#[test]
fn resolves_modes_and_finite_counts() {
    let full = parse_arguments(&arguments(&["full", "--count", "50k"]))
        .unwrap()
        .unwrap();
    assert_eq!(full.expected_total, Some(50_000));
    assert_eq!(full.cycles, 1);
    assert_eq!(full.recipient, "developer@sandpost.test");
    assert_eq!(full.message_format, MessageFormat::Plain);
    let tiny = parse_arguments(&arguments(&["tiny", "--pattern", "50,10,1,50"]))
        .unwrap()
        .unwrap();
    assert_eq!(tiny.cycles, 1);
    assert_eq!(tiny.recipient, "developer@sandpost.test");
    assert_eq!(tiny.message_format, MessageFormat::Plain);
    assert_eq!(tiny.expected_total, Some(111));
    assert!(
        tiny.bursts
            .iter()
            .all(|burst| burst.delay == Duration::from_secs(1))
    );
    let continuous = parse_arguments(&arguments(&[
        "tiny",
        "--pattern",
        "rand(1,2)",
        "--cycles",
        "0",
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(continuous.expected_total, None);
}

/**
 * Accept every template format in both traffic modes.
 */
#[test]
fn resolves_message_formats_in_both_modes() {
    for mode_arguments in [vec!["full", "--count", "1"], vec!["tiny", "--pattern", "1"]] {
        for (format, expected) in [
            ("txt", MessageFormat::Plain),
            ("html", MessageFormat::Html),
            ("newsletter", MessageFormat::Newsletter),
        ] {
            let mut values = mode_arguments.clone();
            values.extend(["--format", format]);
            let options = parse_arguments(&arguments(&values)).unwrap().unwrap();
            assert_eq!(options.message_format, expected);
        }
    }
}

/**
 * Preserve one custom recipient in either traffic mode and reject unsafe or unsupported mailboxes.
 */
#[test]
fn resolves_and_validates_recipients_in_both_modes() {
    for mode_arguments in [vec!["full", "--count", "1"], vec!["tiny", "--pattern", "1"]] {
        for recipient in ["Persona+preview@Sandpost.test", "person@localhost"] {
            let mut values = mode_arguments.clone();
            values.extend(["--to", recipient]);
            let options = parse_arguments(&arguments(&values)).unwrap().unwrap();
            assert_eq!(options.recipient, recipient);
        }
        for recipient in [
            "",
            "missing-domain",
            "@sandpost.test",
            "person@",
            "person@@sandpost.test",
            "person@sandpost.test\r\nBcc: other@sandpost.test",
            "person@sandpost.test\0",
            "person@sandpost.test\t",
            " person@sandpost.test",
            "person@sandpost.test ",
            "Person <person@sandpost.test>",
            "person@sandpost.test,other@sandpost.test",
            "person@sandpost.test;other@sandpost.test",
            "pérson@sandpost.test",
            "person@-sandpost.test",
        ] {
            let mut values = mode_arguments.clone();
            values.extend(["--to", recipient]);
            assert!(parse_arguments(&arguments(&values)).is_err(), "{values:?}");
        }
        let mut missing = mode_arguments.clone();
        missing.push("--to");
        assert!(parse_arguments(&arguments(&missing)).is_err());
        let mut duplicate = mode_arguments;
        duplicate.extend(["--to", "a@sandpost.test", "--to", "b@sandpost.test"]);
        assert!(parse_arguments(&arguments(&duplicate)).is_err());
    }
}

/**
 * Reject incompatible switches, malformed values and finite aggregate overflows before sending.
 */
#[test]
fn rejects_invalid_cli_options() {
    for values in [
        vec!["full"],
        vec!["tiny"],
        vec!["full", "--count", "0"],
        vec!["full", "--count", "1", "--format", "invalid"],
        vec!["full", "--count", "1", "--format", "plain"],
        vec!["tiny", "--pattern", "1", "--format"],
        vec![
            "tiny",
            "--pattern",
            "1",
            "--format",
            "html",
            "--format",
            "plain",
        ],
        vec!["full", "--count", "1", "--cycles", "2"],
        vec!["tiny", "--pattern", "1", "--count", "1"],
        vec!["tiny", "--pattern", "1", "--cycles", "-1"],
        vec!["tiny", "--pattern", "1", "--cycles", "+1"],
        vec!["tiny", "--pattern", "1", "--port", "0"],
        vec!["tiny", "--pattern", "1", "--port", "65536"],
        vec!["tiny", "--pattern", "1", "--host", ""],
        vec!["full", "--count", "1", "--count", "2"],
        vec!["tiny", "--pattern", "18446744073709551615,1"],
        vec!["tiny", "--pattern", "18446744073709551615", "--cycles", "2"],
    ] {
        assert!(parse_arguments(&arguments(&values)).is_err(), "{values:?}");
    }
}

/// Keep sampled random counts within inclusive bounds, including fixed and maximum-width ranges.
#[test]
fn samples_inclusive_random_counts() {
    assert_eq!(sample_quantity(&Quantity::Fixed(50_000)), 50_000);
    assert_eq!(
        sample_quantity(&Quantity::Random {
            minimum: 10,
            maximum: 10
        }),
        10
    );
    for _ in 0..100 {
        assert!((10..=100).contains(&sample_quantity(&Quantity::Random {
            minimum: 10,
            maximum: 100
        })));
        assert!(
            sample_quantity(&Quantity::Random {
                minimum: 1,
                maximum: u64::MAX
            }) >= 1
        );
    }
}

/**
 * Preserve unique message identifiers, a parseable date and per-burst metadata in generated MIME.
 */
#[test]
fn generates_unique_parseable_messages() {
    let first = make_message(2, 3, 4, MessageFormat::Plain, "developer@sandpost.test").unwrap();
    let second = make_message(2, 3, 4, MessageFormat::Plain, "developer@sandpost.test").unwrap();
    let first = mail_parser::MessageParser::default().parse(&first).unwrap();
    let second = mail_parser::MessageParser::default()
        .parse(&second)
        .unwrap();
    assert_ne!(first.message_id(), second.message_id());
    assert!(first.date().is_some());
    assert_eq!(
        first.subject(),
        Some("Sandpost | cycle 2 | burst 3 | email 4")
    );
    assert!(
        first
            .body_text(0)
            .unwrap()
            .contains("cycle 2, burst 3, email 4")
    );
}

/**
 * Keep all template formats safe for SMTP 7bit transport with bounded CRLF lines and no unresolved placeholders.
 */
#[test]
fn generates_messages_with_smtp_safe_lines() {
    for message_format in [
        MessageFormat::Plain,
        MessageFormat::Html,
        MessageFormat::Newsletter,
    ] {
        let message = make_message(2, 3, 4, message_format, "developer@sandpost.test").unwrap();
        assert!(message.is_ascii());
        let message = std::str::from_utf8(&message).unwrap();
        assert!(!message.contains("{{"));
        assert!(!message.replace("\r\n", "").contains(['\r', '\n']));
        assert!(message.split("\r\n").all(|line| line.len() <= 998));
    }
}

/**
 * Render newsletter alternatives with message-specific unsubscribe links and resolved content.
 */
#[test]
fn generates_newsletter_with_unsubscribe_link() {
    let message = make_message(2, 3, 4, MessageFormat::Newsletter, "emanuele@ciao.it").unwrap();
    let message = mail_parser::MessageParser::default()
        .parse(&message)
        .unwrap();
    let identifier = message.message_id().unwrap().split_once('@').unwrap().0;
    assert!(message.is_content_type("multipart", "alternative"));
    assert_eq!(
        message.subject(),
        Some("Sandpost Weekly | A little more care, a better inbox")
    );
    let text = message.body_text(0).unwrap();
    assert!(text.starts_with("Sandpost Weekly"));
    let html = message.body_html(0).unwrap();
    let unsubscribe_url =
        format!("https://example.test/newsletter/unsubscribe?message={identifier}");
    for body in [text.as_ref(), html.as_ref()] {
        assert!(body.contains(&unsubscribe_url));
        assert!(!body.contains("{{"));
    }
    let html_lowercase = html.to_ascii_lowercase();
    assert!(html_lowercase.contains("<h1"));
    assert!(html.contains("Good email starts"));
}

/// An already-requested interruption must skip a long inter-burst pause immediately.
#[test]
fn interruption_skips_long_pauses() {
    assert!(wait_between_bursts(
        Duration::from_secs(600),
        &AtomicBool::new(true)
    ));
}
