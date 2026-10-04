use super::*;
use crate::parser::MAXIMUM_NESTING_DEPTH;

#[test]
/// Verify precedence, literals, and quoted-string escapes during compilation.
fn precedence_literals_and_escapes() {
    let compiled_query = compile("not subject == 'no' and (from.domain == \"example.COM\" or size >= 50) and text contains \"quoted \\\"word\\\"\"").unwrap();
    assert!(compiled_query.evaluate(&facts()));
}

#[test]
/// Verify empty input, boolean constants, and parser nesting limits.
fn empty_constants_and_safety_limits() {
    assert!(compile(" ").unwrap().evaluate(&facts()));
    assert!(compile("true").unwrap().evaluate(&facts()));
    assert!(!compile("false").unwrap().evaluate(&facts()));
    assert!(compile("true == true").is_err());
    let nested = "(".repeat(MAXIMUM_NESTING_DEPTH + 1)
        + "true == true"
        + &")".repeat(MAXIMUM_NESTING_DEPTH + 1);
    assert!(compile(&nested).is_err());
    assert!(compile("subject == \"x\" and").is_err());
}

#[test]
/// Verify malformed and truncated query text never causes a parser panic.
fn malformed_and_truncated_sources_never_panic() {
    let samples = [
        "subject ==",
        "header[",
        "header[\"x\"",
        "header[true]",
        "header[\"x\"]",
        "(",
        "subject",
        ")",
        "and",
        "subject == \"x\" or",
        "from.",
        "envelope.to.domain >",
        "subject === 'x'",
        "subject == 'unterminated",
        "header[\"x\"] ==",
        "((subject == \"x\")",
        "not not",
        "size >= -",
    ];
    for sample in samples {
        for end_byte_position in 0..=sample.len() {
            let prefix = &sample[..end_byte_position];
            assert!(
                std::panic::catch_unwind(|| compile(prefix)).is_ok(),
                "panicked on {prefix:?}"
            );
        }
    }
}
