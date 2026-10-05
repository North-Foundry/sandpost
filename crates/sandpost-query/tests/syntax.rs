mod common;

use sandpost_query::{Expression, Field, Operator, Predicate, Value, compile};

#[test]
/// Verify precedence, case handling, and quoted-string escapes through the public API.
fn precedence_parentheses_case_and_quoted_escapes() {
    let facts = common::facts();
    assert!(
        compile("FALSE or true and false or true")
            .unwrap()
            .evaluate(&facts)
    );
    assert!(
        !compile("(true or true) and false")
            .unwrap()
            .evaluate(&facts)
    );
    assert!(
        compile("not false and (subject == 'Café Hello, World')")
            .unwrap()
            .evaluate(&facts)
    );
    assert!(
        compile(r#"text contains "Straße and 🦀" and html contains "&amp;""#)
            .unwrap()
            .evaluate(&facts)
    );
    assert!(compile(r#"subject == 'Café\'s'"#).is_ok());
    let escaped = sandpost_query::compile(r#"text == "line\nnext\r\t\/""#).unwrap();
    let expected = Expression::Predicate(Predicate {
        field: Field::Text,
        operator: Operator::Equal,
        value: Value::String("line\nnext\r\t/".into()),
    });
    assert_eq!(escaped.expression(), &expected);
    assert_eq!(escaped.into_expression(), expected);
}

#[test]
/// Verify every public field spelling accepts the documented literal/operator types.
fn all_fields_and_literal_operator_types_are_accepted() {
    let accepted = [
        "envelope.from.address == 'sender@example.com'",
        "envelope.from.domain == 'example.com'",
        "envelope.to.address == 'envelope@one.test'",
        "envelope.to.domain == 'one.test'",
        "from.address == 'sender@example.com'",
        "from.domain == 'example.com'",
        "to.address == 'first@one.test'",
        "to.domain == 'one.test'",
        "cc.address == 'copy@three.test'",
        "cc.domain == 'three.test'",
        "subject >= 'Café'",
        "text <= 'Z'",
        "html > 'a'",
        "content contains 'NEEDLE%_'",
        "content >= 'Header'",
        "message_id != 'other'",
        "received_at == -9223372036854775808",
        "size == 9223372036854775807",
        "attachment_count >= 2",
        "has_attachments != false",
        "header['X-Tag'] contains 'Blue'",
        "from == 'sender@example.com'",
        "to == 'first@one.test'",
        "cc == 'copy@three.test'",
    ];
    for source in accepted {
        assert!(compile(source).is_ok(), "rejected valid query {source}");
    }
    for source in [
        "from.domain > 'example.com'",
        "to.address <= 'z'",
        "size contains '1'",
        "has_attachments > true",
        "subject == 1",
        "unknown == 'x'",
        "header[true] == 'x'",
        "subject === 'x'",
        "subject == 'x' trailing",
    ] {
        assert!(compile(source).is_err(), "accepted invalid query {source}");
    }
}

#[test]
/// Verify diagnostics report zero-based byte positions for multibyte source text.
fn errors_report_multibyte_character_byte_positions() {
    let error = compile("subject == 'é' and @").unwrap_err();
    assert_eq!(error.position, "subject == 'é' and ".len());
    let error = compile("subject ==").unwrap_err();
    assert_eq!(error.position, "subject ==".len());
    let error = compile("unknown == 'x'").unwrap_err();
    assert_eq!(error.position, 0);
}

#[test]
/// Verify source, token, and nesting limits at their boundaries.
fn safety_limits_accept_exact_boundaries_and_reject_the_next_byte_or_token() {
    assert!(compile(&" ".repeat(16 * 1024)).is_ok());
    assert_eq!(
        compile(&" ".repeat(16 * 1024 + 1)).unwrap_err().position,
        16 * 1024
    );

    let at_token_limit = std::iter::repeat_n("size==0", 1024)
        .collect::<Vec<_>>()
        .join("or ");
    assert!(at_token_limit.len() < 16 * 1024);
    assert!(compile(&at_token_limit).is_ok()); // 4095 tokens.
    let at_4096 = format!("{at_token_limit} x");
    let error = compile(&at_4096).unwrap_err();
    assert!(!error.message.contains("token count"));
    let at_4097 = format!("{at_4096} x");
    let error = compile(&at_4097).unwrap_err();
    assert!(error.message.contains("token count"));

    assert!(compile(&format!("{}true{}", "(".repeat(64), ")".repeat(64))).is_ok());
    let error = compile(&format!("{}true{}", "(".repeat(65), ")".repeat(65))).unwrap_err();
    assert!(error.message.contains("nesting"));
    assert!(compile(&format!("{}false", "not ".repeat(64))).is_ok());
    assert!(compile(&format!("{}false", "not ".repeat(65))).is_err());
}

#[test]
/// Verify every multibyte-character prefix of malformed input returns without panicking.
fn every_multibyte_character_prefix_of_malformed_inputs_returns_without_panicking() {
    for sample in [
        "subject == '🦀' and @",
        "header['é'] ==",
        "from.domain matches '*é?' and (",
        "text == 'unterminated 🧪",
    ] {
        for end_byte_position in
            (0..=sample.len()).filter(|&byte_index| sample.is_char_boundary(byte_index))
        {
            let prefix = &sample[..end_byte_position];
            assert!(
                std::panic::catch_unwind(|| compile(prefix)).is_ok(),
                "panicked for {prefix:?}"
            );
        }
    }
}
