//! Keep RFC evidence linked to executable tests and emitted statuses to registered assignments.
use std::{collections::BTreeSet, path::Path};

/// Every RFC in the receiver profile must link to existing, enabled tests in its evidence table.
#[test]
fn receiver_rfc_profile_links_to_enabled_tests() {
    let document = include_str!("../RFC-COVERAGE.md");
    let mut covered = BTreeSet::new();
    for section in document.split("\n## RFC ").skip(1) {
        let (number, _) = section.split_once(' ').unwrap();
        let mut evidence_count = 0;
        for row in section.lines().take_while(|line| !line.starts_with("## ")) {
            if !row.starts_with('|') {
                continue;
            }
            for link in row.split('[').skip(1) {
                let Some((name, target)) = link.split_once("](") else {
                    continue;
                };
                let target = target.split(')').next().unwrap();
                if !target.ends_with(".rs") {
                    continue;
                }
                assert!(target.starts_with("src/tests/") || target.starts_with("tests/"));
                let source =
                    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(target))
                        .unwrap();
                let (attributes, _) = source
                    .split_once(&format!("fn {name}("))
                    .unwrap_or_else(|| panic!("RFC {number}: missing {name} in {target}"));
                let attributes = attributes.rsplit("\n\n").next().unwrap();
                assert!(
                    !attributes.contains("#[ignore"),
                    "RFC {number}: {name} is ignored"
                );
                assert!(
                    attributes.contains("#[test]") || attributes.contains("#[tokio::test"),
                    "RFC {number}: {name} must be an enabled test"
                );
                evidence_count += 1;
            }
        }
        assert!(
            evidence_count > 0,
            "RFC {number} has no executable evidence"
        );
        assert!(covered.insert(number), "duplicate RFC {number}");
    }
    assert_eq!(
        covered,
        BTreeSet::from([
            "1870", "2034", "2920", "3207", "3463", "3848", "4616", "4648", "4954", "5248", "5321",
            "5322", "6152", "8314", "8997"
        ])
    );
}

/// RFC 5248: every literal enhanced status emitted by the receiver uses an IANA assignment.
#[test]
fn emitted_enhanced_statuses_use_registered_assignments_and_matching_classes() {
    // IANA SMTP Enhanced Status Codes registry, checked 2026-10-09:
    // https://www.iana.org/assignments/smtp-enhanced-status-codes
    // The registry uses X for the class; the reply determines 2, 4, or 5.
    let assignments = [
        "0.0", "1.0", "1.3", "1.5", "1.7", "3.0", "3.2", "3.4", "4.2", "5.1", "5.2", "5.3", "5.4",
        "5.6", "6.0", "7.0", "7.1", "7.8", "7.9", "7.11",
    ];
    let mut observed = BTreeSet::new();
    for source in [
        include_str!("../src/server.rs"),
        include_str!("../src/session/mod.rs"),
        include_str!("../src/session/authentication.rs"),
        include_str!("../src/session/delivery.rs"),
    ] {
        for literal in source.split('"').skip(1).step_by(2) {
            let mut words = literal.split_ascii_whitespace();
            let Some(primary) = words.next() else {
                continue;
            };
            if primary.len() != 3 || !primary.bytes().all(|byte| byte.is_ascii_digit()) {
                continue;
            }
            let Some(enhanced) = words.next().filter(|word| word.contains('.')) else {
                continue;
            };
            let (class, assignment) = enhanced.split_once('.').unwrap();
            assert_eq!(class, &primary[..1], "{literal}");
            assert!(matches!(class, "2" | "4" | "5"), "{literal}");
            assert!(
                assignments.contains(&assignment),
                "unregistered status: {literal}"
            );
            observed.insert(assignment);
        }
    }
    assert_eq!(observed, BTreeSet::from(assignments));
}
