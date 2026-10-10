use super::*;

/// Verify canonicalization preserves evaluation across generated expressions and facts.
#[test]
/// Verify glob normalization, canonical fingerprints, and mailbox literal normalization.
fn glob_normalization_and_fingerprint() {
    assert!(
        compile("subject matches \"H*?o*\"")
            .unwrap()
            .evaluate(&facts())
    );
    assert_eq!(
        compile("size > 1 and subject == \"Hello, world\"")
            .unwrap()
            .fingerprint(),
        compile("subject == \"Hello, world\" AND size > 1")
            .unwrap()
            .fingerprint()
    );
    assert_eq!(
        compile("size > 1 and (subject == \"Hello, world\" and text contains \"quoted\")")
            .unwrap()
            .fingerprint(),
        compile("text contains \"quoted\" and size > 1 and subject == \"Hello, world\"")
            .unwrap()
            .fingerprint()
    );
    assert_eq!(
        compile("not not subject == \"Hello, world\"")
            .unwrap()
            .fingerprint(),
        compile("subject == \"Hello, world\"")
            .unwrap()
            .fingerprint()
    );
    assert!(matches!(
        compile("subject == \"Hello, world\" and not subject == \"Hello, world\"")
            .unwrap()
            .into_expression(),
        Expression::False
    ));
    assert!(!compile("not true and false").unwrap().evaluate(&facts()));
    assert!(
        !compile("false or true and false")
            .unwrap()
            .evaluate(&facts())
    );
    assert!(
        compile("true or false and false")
            .unwrap()
            .evaluate(&facts())
    );
    let Expression::Predicate(p) = &compile("from.address == \"A@EXAMPLE.COM\"")
        .unwrap()
        .into_expression()
    else {
        panic!()
    };
    assert_eq!(p.value, Value::String("a@example.com".into()));
    assert_eq!(
        Field::FromAddress.values(&facts()),
        vec![Value::String("a@example.com".into())]
    );
    let mut with_star = facts();
    with_star.text = "*xa".into();
    assert!(compile("text matches \"*a\"").unwrap().evaluate(&with_star));
    assert_eq!(compile("a").unwrap_err().position, 0);
}

#[test]
/// Verify invalid types fail and boolean complements simplify exactly.
fn type_errors_and_boolean_simplification() {
    for source in [
        "size contains 'x'",
        "has_attachments > false",
        "from.domain == 12",
        "unknown_field == 'x'",
        "subject == true",
    ] {
        assert!(compile(source).is_err(), "accepted {source}");
    }

    let subject_query = "subject == 'Hello, world'";
    let negated_subject_query = format!("not ({subject_query})");
    for equivalent_query in [
        format!("{subject_query} and true"),
        format!("not not ({subject_query})"),
    ] {
        assert_eq!(
            compile(subject_query).unwrap().into_expression(),
            compile(&equivalent_query).unwrap().into_expression()
        );
    }
    assert_eq!(
        compile(&format!("{subject_query} and {negated_subject_query}"))
            .unwrap()
            .into_expression(),
        Expression::False
    );
    assert_eq!(
        compile(&format!("{subject_query} or {negated_subject_query}"))
            .unwrap()
            .into_expression(),
        Expression::True
    );

    let mixed = facts();
    assert!(
        compile("to.address == 'b@test.org'")
            .unwrap()
            .evaluate(&mixed)
    );
    assert!(
        compile("to.address != 'b@test.org'")
            .unwrap()
            .evaluate(&mixed)
    );
}

/// Composed expressions fingerprint like their compiled equivalent, independent of order.
#[test]
fn composed_expressions_share_compiled_fingerprints() {
    let left = crate::compile("subject contains \"a\"")
        .unwrap()
        .into_expression();
    let right = crate::compile("size > 10").unwrap().into_expression();
    let (_, composed) =
        Expression::And(vec![left.clone(), right.clone()]).into_canonical_with_fingerprint();
    let (_, reordered) = Expression::And(vec![right, left]).into_canonical_with_fingerprint();
    let compiled = crate::compile("size > 10 and subject contains \"a\"").unwrap();
    assert_eq!(composed, reordered);
    assert_eq!(composed, compiled.fingerprint());
}
