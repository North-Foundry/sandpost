mod common;

use sandpost_query::{Field, Value, compile};

#[test]
/// Verify comparison operators across scalar types and glob matching.
fn comparison_operators_cover_strings_numbers_booleans_and_globs() {
    let facts = common::facts();
    for (query, expected) in [
        ("subject == 'Café Hello, World'", true),
        ("subject != 'other'", true),
        ("subject > 'Café'", true),
        ("subject >= 'Café Hello, World'", true),
        ("subject < 'Z'", true),
        ("subject <= 'Café Hello, World'", true),
        ("text contains 'Straße'", true),
        ("text starts_with 'Body'", true),
        ("text ends_with '🦀'", true),
        ("text matches 'Body*🦀'", true),
        ("text matches 'body*🦀'", false),
        ("html contains 'Bright'", true),
        ("content contains 'BODY'", true),
        ("content contains 'X-TAG'", true),
        ("content contains 'GREEN'", true),
        ("content contains 'Café'", true),
        ("content contains 'CAFÉ'", false),
        ("content contains '50%_'", true),
        ("content contains '50%X'", false),
        ("content matches 'BODY*🦀'", true),
        ("message_id contains 'msg.1'", true),
        ("header['subject'] > 'Header'", true),
        ("header['x-tag'] == 'green'", true),
        ("received_at == -9223372036854775808", true),
        ("received_at < 9223372036854775807", true),
        ("received_at <= -9223372036854775808", true),
        ("received_at > -9223372036854775808", false),
        ("received_at >= -9223372036854775808", true),
        ("received_at != 0", true),
        ("size == 9223372036854775807", false),
        ("size > 9223372036854775807", true),
        ("size >= 9223372036854775807", true),
        ("size < 9223372036854775807", false),
        ("size <= 9223372036854775807", false),
        ("size != 9223372036854775807", true),
        ("attachment_count == 2", true),
        ("attachment_count != 2", false),
        ("attachment_count > 1", true),
        ("attachment_count >= 2", true),
        ("attachment_count < 3", true),
        ("attachment_count <= 2", true),
        ("has_attachments == true", true),
        ("has_attachments != false", true),
    ] {
        assert_eq!(
            compile(query).unwrap().evaluate(&facts),
            expected,
            "{query}"
        );
    }
    assert!(compile("received_at == 9223372036854775808").is_err());
    assert!(compile("received_at == -9223372036854775809").is_err());
}

#[test]
/// Verify Content values include all supported literal fields in ASCII-lowercase form.
fn content_field_collects_normalized_subject_body_mailbox_and_headers() {
    let mut facts = common::facts();
    facts.envelope_from = Some(sandpost_core::Mailbox {
        address: "Env-Sender@Example.test".into(),
        domain: "example.test".into(),
    });
    facts.envelope_to.push(sandpost_core::Mailbox {
        address: "Env-Recipient@Example.test".into(),
        domain: "example.test".into(),
    });
    facts.carbon_copy.push(sandpost_core::Mailbox {
        address: "Copy@Three.test".into(),
        domain: "three.test".into(),
    });
    facts.message_identifier = Some("<Message-ID@example.test>".into());
    facts
        .headers
        .get_mut("x-tag")
        .unwrap()
        .push("literal 50%_ marker".into());
    let content = Field::Content.values(&facts);
    for value in [
        "café hello, world",
        "body has straße and 🦀",
        "env-sender@example.test",
        "env-recipient@example.test",
        "sender@example.com",
        "first@one.test",
        "second@two.test",
        "copy@three.test",
        "<message-id@example.test>",
        "x-tag",
        "blue",
        "green",
    ] {
        assert!(content.contains(&Value::String(value.into())), "{value}");
    }
    assert!(
        compile("content >= 'CAFé HELLO, WORLD'")
            .unwrap()
            .evaluate(&facts)
    );
    assert!(
        compile("content == 'CAFé HELLO, WORLD'")
            .unwrap()
            .evaluate(&facts)
    );
    assert!(
        compile("content contains 'X-TAG'")
            .unwrap()
            .evaluate(&facts)
    );
    assert!(
        compile("content contains '50%_' ")
            .unwrap()
            .evaluate(&facts)
    );
    assert!(!compile("content contains '50%X'").unwrap().evaluate(&facts));
}

#[test]
/// Verify mailbox address and domain comparisons ignore basic Latin letter case.
fn every_mailbox_family_uses_basic_latin_case_insensitive_comparisons() {
    let facts = common::facts();
    for query in [
        "envelope.from.address == 'SENDER@EXAMPLE.COM'",
        "envelope.from == 'SENDER@EXAMPLE.COM'",
        "envelope.from.domain contains 'EXAMPLE'",
        "envelope.to.address starts_with 'ENVELOPE@'",
        "envelope.to == 'ENVELOPE@ONE.TEST'",
        "envelope.to.domain ends_with 'TEST'",
        "from.address matches 'SENDER@EXAMPLE.COM'",
        "from.domain == 'EXAMPLE.COM'",
        "to.address == 'SECOND@TWO.TEST'",
        "to.domain == 'TWO.TEST'",
        "cc.address == 'COPY@THREE.TEST'",
        "cc.domain == 'THREE.TEST'",
    ] {
        assert!(compile(query).unwrap().evaluate(&facts), "{query}");
    }
    assert!(compile("subject contains 'Café'").unwrap().evaluate(&facts));
    assert!(!compile("subject contains 'café'").unwrap().evaluate(&facts));
    assert!(
        compile("subject matches 'Café*' ")
            .unwrap()
            .evaluate(&facts)
    );
    assert!(!compile("subject matches 'café*'").unwrap().evaluate(&facts));

    let largest = sandpost_core::MessageFacts {
        size: u64::MAX,
        attachment_count: u64::MAX,
        ..Default::default()
    };
    assert!(
        compile("size > 9223372036854775807")
            .unwrap()
            .evaluate(&largest)
    );
    assert!(
        compile("attachment_count > 9223372036854775807")
            .unwrap()
            .evaluate(&largest)
    );
}

#[test]
/// Verify collection inequality is existential and negation wraps the whole collection.
fn collection_inequality_is_any_and_negation_wraps_the_collection() {
    let facts = common::facts();
    assert!(compile("to.domain == 'one.test'").unwrap().evaluate(&facts));
    assert!(compile("to.domain != 'one.test'").unwrap().evaluate(&facts));
    assert!(
        !compile("not (to.domain == 'one.test')")
            .unwrap()
            .evaluate(&facts)
    );
    assert!(
        compile("not (to.domain == 'absent.test')")
            .unwrap()
            .evaluate(&facts)
    );
    assert!(
        compile("header['x-tag'] != 'Blue'")
            .unwrap()
            .evaluate(&facts)
    );
    assert!(
        !compile("not (header['x-tag'] == 'Blue')")
            .unwrap()
            .evaluate(&facts)
    );
}

#[test]
/// Verify absent optional fields and empty collections match no comparisons.
fn optional_and_collection_fields_match_nothing_when_absent() {
    let facts = sandpost_core::MessageFacts::default();
    for query in [
        "envelope.from.address == 'x@example.com'",
        "envelope.from.domain != 'x'",
        "envelope.to.address == 'x'",
        "from.domain contains 'x'",
        "to.address != 'x'",
        "cc.domain matches '*'",
        "message_id != 'x'",
        "header['x-missing'] != 'x'",
    ] {
        assert!(!compile(query).unwrap().evaluate(&facts), "{query}");
    }
    assert!(compile("not (message_id == 'x')").unwrap().evaluate(&facts));
    assert!(
        compile("not (header['x-missing'] == 'x')")
            .unwrap()
            .evaluate(&facts)
    );
}

#[test]
/// Verify an empty glob pattern matches only an empty scalar string.
fn empty_glob_pattern_only_matches_empty_string_and_empty_text() {
    let mut facts = common::facts();
    assert!(!compile("text matches ''").unwrap().evaluate(&facts));
    assert!(compile("not (text matches '')").unwrap().evaluate(&facts));
    facts.text.clear();
    assert!(compile("text matches ''").unwrap().evaluate(&facts));
    assert!(compile("text contains ''").unwrap().evaluate(&facts));
    assert!(!compile("not (text contains '')").unwrap().evaluate(&facts));
}

#[test]
/// Verify negative integer literals compare exactly with unsigned fact fields.
fn negative_literals_compare_exactly_against_unsigned_fields() {
    let facts = sandpost_core::MessageFacts::default();
    for field in ["size", "attachment_count"] {
        for (operator, expected) in [
            ("==", false),
            ("!=", true),
            (">", true),
            (">=", true),
            ("<", false),
            ("<=", false),
        ] {
            let query = format!("{field} {operator} -1");
            assert_eq!(
                compile(&query).unwrap().evaluate(&facts),
                expected,
                "{query}"
            );
        }
    }
    let largest = sandpost_core::MessageFacts {
        size: u64::MAX,
        attachment_count: u64::MAX,
        ..Default::default()
    };
    for field in ["size", "attachment_count"] {
        assert!(
            !compile(&format!("{field} == -1"))
                .unwrap()
                .evaluate(&largest)
        );
        assert!(
            compile(&format!("{field} > -1"))
                .unwrap()
                .evaluate(&largest)
        );
    }
}
