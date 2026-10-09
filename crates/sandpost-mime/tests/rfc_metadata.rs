//! Keep the MIME receiver profile's evidence linked to enabled integration tests.

use std::{collections::BTreeSet, fs, path::PathBuf};

/// Ensure every RFC profile section cites an existing, enabled integration test.
#[test]
fn rfc_profile_rows_name_existing_enabled_integration_tests() {
    let manifest_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let profile = include_str!("../RFC-COVERAGE.md");
    let mut seen = BTreeSet::new();
    let sections = profile
        .split("\n## RFC ")
        .skip(1)
        .filter(|section| section.starts_with(|character: char| character.is_ascii_digit()))
        .collect::<Vec<_>>();
    assert!(!sections.is_empty(), "RFC profile has no sections");

    for section in sections {
        let number = section
            .split_once(' ')
            .expect("RFC section heading includes a number")
            .0
            .trim_end_matches(|character: char| !character.is_ascii_digit());
        assert!(
            seen.insert(number.to_owned()),
            "duplicate RFC profile section {number}"
        );

        let evidence = section
            .lines()
            .filter_map(|line| line.strip_prefix("Evidence: "))
            .flat_map(|line| line.split('`').skip(1).step_by(2))
            .collect::<Vec<_>>();
        assert!(!evidence.is_empty(), "RFC {number} has no test evidence");

        for reference in evidence {
            let (file_name, function_name) = reference
                .split_once("::")
                .unwrap_or_else(|| panic!("RFC {number}: malformed evidence `{reference}`"));
            let source = fs::read_to_string(manifest_directory.join("tests").join(file_name))
                .unwrap_or_else(|error| panic!("RFC {number}: cannot read {file_name}: {error}"));
            assert_enabled_test(&source, function_name, number, file_name);
        }
    }

    assert_eq!(
        seen,
        [
            "2045", "2046", "2047", "2048", "2049", "2183", "2231", "2387", "2392", "5322", "6532",
            "6533", "6838"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        "RFC profile sections changed"
    );
}

/// Require the named function to have an enabled `#[test]` attribute immediately above it.
fn assert_enabled_test(source: &str, function_name: &str, rfc: &str, file_name: &str) {
    let declaration = format!("fn {function_name}(");
    let lines = source.lines().collect::<Vec<_>>();
    let index = lines
        .iter()
        .position(|line| line.trim_start().starts_with(&declaration))
        .unwrap_or_else(|| panic!("RFC {rfc}: missing {function_name} in {file_name}"));
    let test_attribute = lines[..index]
        .iter()
        .rposition(|line| line.trim() == "#[test]")
        .unwrap_or_else(|| panic!("RFC {rfc}: {function_name} is not a test"));
    let mut attributes_start = test_attribute;
    while attributes_start > 0 && lines[attributes_start - 1].trim_start().starts_with("#[") {
        attributes_start -= 1;
    }

    assert!(
        lines[attributes_start..index]
            .iter()
            .all(|line| !line.trim_start().starts_with("#[ignore")),
        "RFC {rfc}: {function_name} is ignored"
    );
    assert!(
        lines[test_attribute + 1..index]
            .iter()
            .all(|line| line.trim().is_empty() || line.trim_start().starts_with("///")),
        "RFC {rfc}: {function_name} is not directly attached to #[test]"
    );
}
