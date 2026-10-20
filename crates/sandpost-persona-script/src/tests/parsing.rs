use super::parse_error;
use crate::{Alternative, LinkSelector, ScriptLimits, StatementKind, compile, parse};
use std::time::Duration;

#[test]
/**
 * Verify a sequential script parses into typed statements with source spans.
 */
fn sequential_statements_parse_in_order() {
    let script = parse(
        "wait 5s;\nopen;\nwait 500ms; wait 2m;\nclick link(\"a.verify\");",
        &ScriptLimits::default(),
    )
    .unwrap();
    let kinds: Vec<_> = script
        .statements
        .iter()
        .map(|statement| &statement.kind)
        .collect();
    assert_eq!(kinds[0], &StatementKind::Wait(Duration::from_secs(5)));
    assert_eq!(kinds[1], &StatementKind::Open);
    assert_eq!(kinds[2], &StatementKind::Wait(Duration::from_millis(500)));
    assert_eq!(kinds[3], &StatementKind::Wait(Duration::from_secs(120)));
    let StatementKind::Click(LinkSelector::Css(selector)) = kinds[4] else {
        panic!("expected a CSS link selector");
    };
    assert_eq!(selector.source(), "a.verify");
    let open = &script.statements[1].span;
    assert_eq!(
        (open.line, open.column, open.start, open.end),
        (2, 1, 9, 14)
    );
    assert_eq!(script.statements[4].span.line, 4);
}

#[test]
/**
 * Verify `if`, `else if`, `else`, and nested blocks build the expected tree.
 */
fn nested_conditionals_with_else_if_and_else() {
    let source = r#"
        if (subject contains "Invoice") {
            open;
            if (from.domain == "example.com") { click link(text: "Pay"); }
        } else if (subject contains "Newsletter") {
            wait 30s;
            open;
        } else {
            wait 10s;
        }
    "#;
    let script = parse(source, &ScriptLimits::default()).unwrap();
    assert_eq!(script.statements.len(), 1);
    let StatementKind::If(outer) = &script.statements[0].kind else {
        panic!("expected if");
    };
    assert_eq!(outer.condition.source, r#"subject contains "Invoice""#);
    assert_eq!(outer.consequence.len(), 2);
    assert!(matches!(outer.consequence[1].kind, StatementKind::If(_)));
    let Some(Alternative::ElseIf(else_if)) = &outer.alternative else {
        panic!("expected else if");
    };
    let StatementKind::If(middle) = &else_if.kind else {
        panic!("expected nested if");
    };
    assert_eq!(middle.consequence.len(), 2);
    let Some(Alternative::Else(otherwise)) = &middle.alternative else {
        panic!("expected else");
    };
    assert_eq!(
        otherwise[0].kind,
        StatementKind::Wait(Duration::from_secs(10))
    );
}

#[test]
/**
 * Verify conditions accept the full Sandpost query language, compiled by `sandpost-query`.
 */
fn conditions_use_existing_query_expressions() {
    let conditions = [
        r#"from.domain == "example.com""#,
        r#"subject contains "Welcome" and not (to.address matches "*@spam.test")"#,
        r#"header["X-App"] == "bravo" or size >= 1024"#,
        "has_attachments == false and attachment_count < 3",
        r#"envelope.to.domain == "ecommerce.it" and received_at >= 0"#,
        r#"(subject == "a ) b" or text contains "{")"#,
        "true",
    ];
    for condition in conditions {
        let source = format!("if ({condition}) {{ open; }}");
        let script = parse(&source, &ScriptLimits::default())
            .unwrap_or_else(|error| panic!("{condition}: {error}"));
        let StatementKind::If(conditional) = &script.statements[0].kind else {
            panic!("expected if");
        };
        assert_eq!(
            conditional.condition.query,
            sandpost_query::compile(condition).unwrap(),
            "the condition is exactly the standalone query"
        );
    }
}

#[test]
/**
 * Verify all three link selector forms and their display syntax.
 */
fn link_selector_forms() {
    let script = parse(
        r#"click link("a.verify"); click link(text: "Confirm"); click link(href: "*unsubscribe*");"#,
        &ScriptLimits::default(),
    )
    .unwrap();
    let rendered: Vec<String> = script
        .statements
        .iter()
        .map(|statement| match &statement.kind {
            StatementKind::Click(selector) => selector.to_string(),
            _ => panic!("expected click"),
        })
        .collect();
    assert_eq!(
        rendered,
        [
            r#"link("a.verify")"#,
            r#"link(text: "Confirm")"#,
            r#"link(href: "*unsubscribe*")"#
        ]
    );
}

#[test]
/**
 * Verify comments and arbitrary whitespace are ignored.
 */
fn comments_and_whitespace_are_ignored() {
    let script = parse(
        "// greet\nopen; /* then\n wait */ wait 1s;\n\n\t// done",
        &ScriptLimits::default(),
    )
    .unwrap();
    assert_eq!(script.statements.len(), 2);
    assert!(
        parse("", &ScriptLimits::default())
            .unwrap()
            .statements
            .is_empty()
    );
}

#[test]
/**
 * Verify unknown and misplaced instructions report their exact line and column.
 */
fn invalid_statements_report_positions() {
    let error = parse_error("open;\nwait 1s;\n    jump;");
    assert_eq!((error.line, error.column, error.position), (3, 5, 19));
    assert!(
        error.message.contains("unknown instruction `jump`"),
        "{}",
        error.message
    );
    assert!(error.to_string().starts_with("line 3, column 5: "));

    let cases = [
        ("else { open; }", "`else` must directly follow"),
        ("open;;", "empty statements are not allowed"),
        ("{ open; }", "a block can only follow"),
        ("open", "expected `;` after `open`"),
        ("open wait 1s;", "expected `;` after `open`"),
        ("Open;", "unknown instruction `Open`"),
        ("\"open\";", "expected an instruction"),
        ("open; }", "unexpected `}`"),
        ("open; @", "unexpected character `@`"),
    ];
    for (source, expected) in cases {
        let error = parse_error(source);
        assert!(
            error.message.contains(expected),
            "{source:?}: {}",
            error.message
        );
    }
}

#[test]
/**
 * Verify argument validation for waits and link selectors.
 */
fn invalid_arguments_are_rejected() {
    let cases = [
        ("wait 5;", "needs a unit", 5),
        ("wait 5h;", "unknown duration unit `h`", 6),
        ("wait;", "expected a duration", 4),
        ("wait 11m;", "exceeds the maximum of 10m", 5),
        ("wait 99999999999999999999s;", "too large", 5),
        ("click link();", "needs a CSS selector", 11),
        ("click link(foo: \"x\");", "unknown link argument `foo`", 11),
        ("click link(\"a\", \"b\");", "exactly one argument", 14),
        ("click link(\"  \");", "must not be empty", 11),
        ("click link(\"a[\");", "invalid CSS selector", 11),
        ("click \"a\";", "expected `link(...)`", 6),
        (
            "click link(text: 5s);",
            "expected a quoted string after `text:`",
            17,
        ),
        ("click link(text \"x\");", "expected `:` after `text`", 16),
        ("click link(\"a\";", "expected `)`", 14),
        ("click link;", "expected `(` after `link`", 10),
    ];
    for (source, expected, position) in cases {
        let error = parse_error(source);
        assert!(
            error.message.contains(expected),
            "{source:?}: {}",
            error.message
        );
        assert_eq!(error.position, position, "{source:?}: {}", error.message);
    }
}

#[test]
/**
 * Verify malformed blocks and conditions are rejected with useful positions.
 */
fn malformed_blocks_and_conditions() {
    let error = parse_error("if (subject == \"x\") open;");
    assert!(
        error
            .message
            .contains("expected `{` to start the `if` block")
    );
    let error = parse_error("if subject == \"x\" { open; }");
    assert!(error.message.contains("expected `(` after `if`"));
    let error = parse_error("if (true) {\n  open;\n");
    assert!(
        error
            .message
            .contains("close the block opened at line 1, column 11"),
        "{}",
        error.message
    );
    assert_eq!(error.line, 3);
    let error = parse_error("if (true) { open; } else open;");
    assert!(
        error
            .message
            .contains("expected `{` to start the `else` block")
    );
    let error = parse_error("click link(\"a);");
    assert!(error.message.contains("unterminated string"));
    let error = parse_error("open; /* never closed");
    assert!(error.message.contains("unterminated `/*` comment"));

    let error = parse_error("open;\nif (subjekt == \"x\") { open; }");
    assert_eq!((error.line, error.column), (2, 5));
    assert_eq!(error.message, "invalid condition: unknown field");
    let error = parse_error("if (subject == 5) { open; }");
    assert!(
        error
            .message
            .starts_with("invalid condition: operator and literal type")
    );
    let error = parse_error("if () { open; }");
    assert!(error.message.contains("expected field name"));
    let error = parse_error("if (subject == \"x\" { open; }");
    assert!(error.message.starts_with("invalid condition"));
}

#[test]
/**
 * Verify columns count characters rather than bytes on lines with non-ASCII text.
 */
fn columns_count_characters() {
    let error = parse_error("// café ☕\nif (subject == \"é\") { öpen; }");
    assert_eq!(error.line, 2);
    assert_eq!(error.column, 23);
    assert!(error.message.contains("unexpected character `ö`"));
}

#[test]
/**
 * Verify the size, statement, nesting, and total wait limits.
 */
fn complexity_limits_are_enforced() {
    let limits = ScriptLimits {
        maximum_source_bytes: 64,
        maximum_statements: 3,
        maximum_nesting_depth: 2,
        maximum_wait: Duration::from_secs(60),
        maximum_total_wait: Duration::from_secs(90),
    };
    let error = parse(&"open;".repeat(20), &limits).unwrap_err();
    assert!(error.message.contains("maximum size of 64 bytes"));
    let error = parse("open; open; open; open;", &limits).unwrap_err();
    assert!(error.message.contains("maximum of 3 statements"));
    assert_eq!(error.position, 18);
    let error = parse("if (true) { if (true) { if (true) {} } }", &limits).unwrap_err();
    assert!(error.message.contains("maximum of 2 levels"));
    assert!(parse("if (true) { if (true) {} }", &limits).is_ok());
    let error = parse(
        "if (true) { } else if (true) { } else if (true) { }",
        &limits,
    );
    assert!(
        error.is_ok(),
        "else-if chains do not deepen nesting: {error:?}"
    );
    let roomy = ScriptLimits {
        maximum_source_bytes: 1024,
        ..limits.clone()
    };
    let error = parse(
        "if (true) {} else if (true) {} else if (true) {} else if (true) {}",
        &roomy,
    )
    .unwrap_err();
    assert!(error.message.contains("maximum of 3 statements"));
    let waits = ScriptLimits {
        maximum_statements: 100,
        ..roomy
    };
    let error = compile("wait 60s; wait 31s;", &waits).unwrap_err();
    assert!(
        error.message.contains("can wait 91s in total"),
        "{}",
        error.message
    );
    assert!(
        compile(
            "wait 60s; if (true) { wait 30s; } else { wait 20s; }",
            &waits
        )
        .is_ok()
    );
}

#[test]
/**
 * Verify arbitrary truncations of valid scripts never panic the parser.
 */
fn truncated_sources_never_panic() {
    let source = "wait 5s;\nopen; // é\nif (subject contains \"Verify\" and (size > 1)) {\n  click link(text: \"Conf\\\"irm\");\n} else if (true) { wait 2m; } else { click link(href: \"*x*\"); }";
    for end in 0..=source.len() {
        if source.is_char_boundary(end) {
            let prefix = &source[..end];
            let _ = parse(prefix, &ScriptLimits::default());
        }
    }
}
