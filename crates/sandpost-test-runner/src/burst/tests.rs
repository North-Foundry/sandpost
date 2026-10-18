use super::*;

/// Build native CLI arguments without changing process environment during parallel tests.
fn arguments(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

/// Verify full shorthand and default finite tiny patterns are resolved before network activity.
#[test]
fn resolves_modes_and_finite_counts() {
    let full = parse_arguments(&arguments(&["full", "--count", "50k"]))
        .unwrap()
        .unwrap();
    assert_eq!(full.expected_total, Some(50_000));
    assert_eq!(full.cycles, 1);
    let tiny = parse_arguments(&arguments(&["tiny", "--pattern", "50,10,1,50"]))
        .unwrap()
        .unwrap();
    assert_eq!(tiny.cycles, 1);
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

/// Reject incompatible switches, malformed values and finite aggregate overflows before sending.
#[test]
fn rejects_invalid_cli_options() {
    for values in [
        vec!["full"],
        vec!["tiny"],
        vec!["full", "--count", "0"],
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

/// Preserve unique message identifiers, a parseable date and per-burst metadata in generated MIME.
#[test]
fn generates_unique_parseable_messages() {
    let first = make_message(2, 3, 4).unwrap();
    let second = make_message(2, 3, 4).unwrap();
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

/// An already-requested interruption must skip a long inter-burst pause immediately.
#[test]
fn interruption_skips_long_pauses() {
    assert!(wait_between_bursts(
        Duration::from_secs(600),
        &AtomicBool::new(true)
    ));
}
