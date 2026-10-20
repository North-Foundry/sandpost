mod common;

use sandpost_query::{compile, compile_embedded, glob_matches};

#[test]
/**
 * Verify an embedded condition ends at its closing parenthesis and matches standalone compilation.
 */
fn embedded_condition_matches_standalone_compilation() {
    let source = r#"if (subject contains "Hello" and (size > 1 or false)) { open; }"#;
    let start = source.find('(').unwrap() + 1;
    let embedded = compile_embedded(source, start).unwrap();
    assert_eq!(&source[embedded.end..embedded.end + 1], ")");
    assert_eq!(
        &source[embedded.end..],
        ") { open; }",
        "the host resumes after the closing parenthesis"
    );
    let standalone = compile(r#"subject contains "Hello" and (size > 1 or false)"#).unwrap();
    assert_eq!(embedded.query, standalone);
    assert!(embedded.query.evaluate(&common::facts()));
}

#[test]
/**
 * Verify parentheses and braces inside string literals are content rather than delimiters.
 */
fn embedded_parentheses_inside_strings_are_content() {
    let source = r#"(subject == "a ) b" or subject == 'c ( d') rest"#;
    let embedded = compile_embedded(source, 1).unwrap();
    assert_eq!(&source[embedded.end..], ") rest");
}

#[test]
/**
 * Verify embedded diagnostics report absolute byte positions within the host source.
 */
fn embedded_errors_use_host_positions() {
    let source = "wait 1s;\nif (subjekt == \"x\") {}";
    let start = source.find('(').unwrap() + 1;
    let error = compile_embedded(source, start).unwrap_err();
    assert_eq!(error.position, start);
    assert_eq!(error.message, "unknown field");

    let unterminated = "if (subject == \"x\" {";
    let error = compile_embedded(unterminated, 4).unwrap_err();
    assert_eq!(error.message, "unexpected character");
    assert_eq!(error.position, unterminated.find('{').unwrap());

    let missing = "if (subject == \"x\"";
    let error = compile_embedded(missing, 4).unwrap_err();
    assert_eq!(error.position, missing.len());
}

#[test]
/**
 * Verify an empty embedded condition is rejected instead of silently accepting every message.
 */
fn empty_embedded_condition_is_rejected() {
    let error = compile_embedded("if (  ) {}", 4).unwrap_err();
    assert_eq!(error.position, 6);
    assert_eq!(error.message, "expected field name");
}

#[test]
/**
 * Verify an invalid start offset is reported rather than panicking.
 */
fn invalid_embedded_start_is_an_error() {
    assert!(compile_embedded("é", 1).is_err());
    assert!(compile_embedded("abc", 10).is_err());
}

#[test]
/**
 * Verify the public glob matcher is the `matches` operator's whole-value semantics.
 */
fn public_glob_matches_query_semantics() {
    assert!(glob_matches(
        "https://a.test/unsubscribe?x=1",
        "*unsubscribe*",
        false
    ));
    assert!(!glob_matches(
        "https://a.test/UNSUBSCRIBE",
        "*unsubscribe*",
        false
    ));
    assert!(glob_matches(
        "https://a.test/UNSUBSCRIBE",
        "*unsubscribe*",
        true
    ));
    assert!(glob_matches("", "", false));
    assert!(!glob_matches("x", "", false));
    assert!(glob_matches("ab", "a?", false));
}
