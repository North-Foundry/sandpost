mod common;

use sandpost_query::{Expression, Field, Operator, Predicate, Value, compile};

/// Construct a typed predicate expression for canonicalization cases.
fn predicate_expression(field: Field, operator: Operator, value: Value) -> Expression {
    Expression::Predicate(Predicate {
        field,
        operator,
        value,
    })
}

/// Generate bounded combinations of expressions for semantic equivalence checks.
fn generate_expression_variants(
    expressions: Vec<Expression>,
    remaining_depth: usize,
) -> Vec<Expression> {
    if remaining_depth == 0 {
        return expressions;
    }
    let mut generated = expressions.clone();
    for expression in &expressions {
        generated.push(Expression::Not(Box::new(expression.clone())));
    }
    for left_expression in &expressions {
        for right_expression in &expressions {
            generated.push(Expression::And(vec![
                left_expression.clone(),
                right_expression.clone(),
            ]));
            generated.push(Expression::Or(vec![
                left_expression.clone(),
                right_expression.clone(),
            ]));
        }
    }
    generate_expression_variants(generated, remaining_depth - 1)
}

#[test]
/// Verify canonicalization preserves results over generated expressions and fact fixtures.
fn canonicalization_preserves_semantics_for_generated_expressions_and_facts() {
    let subject_expression = predicate_expression(
        Field::Subject,
        Operator::Contains,
        Value::String("a".into()),
    );
    let size_expression =
        predicate_expression(Field::Size, Operator::GreaterThan, Value::Number(10));
    let recipient_expression = predicate_expression(
        Field::ToDomain,
        Operator::NotEqual,
        Value::String("one.test".into()),
    );
    let expressions = generate_expression_variants(
        vec![
            subject_expression,
            size_expression,
            recipient_expression,
            Expression::True,
            Expression::False,
        ],
        2,
    );
    let mut facts = vec![common::facts(), sandpost_core::MessageFacts::default()];
    let mut altered = common::facts();
    altered.subject.clear();
    altered.size = 0;
    altered.to.clear();
    facts.push(altered);

    for expression in expressions {
        let canonical = expression.clone().canonicalize();
        assert_eq!(canonical.clone().canonicalize(), canonical);
        for facts in &facts {
            assert_eq!(expression.evaluate(facts), canonical.evaluate(facts));
        }
    }
}

#[test]
/// Verify operand ordering, idempotence, and exact expression complements are stable.
fn commutation_idempotence_and_exact_complement_identities_are_stable() {
    let subject_query = "subject contains 'Hello'";
    let size_query = "size > 10";
    let combined_query = compile(&format!("{subject_query} and {size_query}")).unwrap();
    assert_eq!(
        combined_query.fingerprint(),
        compile(&format!("{size_query} and {subject_query}"))
            .unwrap()
            .fingerprint()
    );
    assert_eq!(
        combined_query.fingerprint(),
        compile(&format!(
            "{subject_query} and {subject_query} and {size_query}"
        ))
        .unwrap()
        .fingerprint()
    );
    assert_eq!(
        compile(&format!("{subject_query} or {size_query}"))
            .unwrap()
            .fingerprint(),
        compile(&format!("{size_query} or {subject_query}"))
            .unwrap()
            .fingerprint()
    );
    assert_eq!(
        compile(&format!("not not ({subject_query})"))
            .unwrap()
            .fingerprint(),
        compile(subject_query).unwrap().fingerprint()
    );
    assert_eq!(
        compile(&format!("{subject_query} and not ({subject_query})"))
            .unwrap()
            .expression(),
        &Expression::False
    );
    assert_eq!(
        compile(&format!("{subject_query} or not ({subject_query})"))
            .unwrap()
            .expression(),
        &Expression::True
    );

    // Collection inequality is existential and therefore is not an equality complement.
    let facts = common::facts();
    assert!(compile("to.domain == 'one.test'").unwrap().evaluate(&facts));
    assert!(compile("to.domain != 'one.test'").unwrap().evaluate(&facts));
}

#[test]
/// Verify the unchanged v1 encoding against fingerprints recorded before the refactor.
fn format_v1_fingerprint_matches_pre_refactor_goldens() {
    for (source, expected) in [
        (
            "false",
            "a7b8d8e4206ca55761252e8aadb880b2d19431cd42252e636acbbbd8f13d95fc",
        ),
        (
            "size >= -1",
            "c3fc073b10a681e4cce90d8db37269106ffc4a950d2939d953212cc715732708",
        ),
        (
            "has_attachments != false",
            "9b945ff96ee1dd7ee0e7acd324e21e3c2bb496993ce68e7fda4cb40c4f0602b3",
        ),
        (
            "header[\"X-App\"] == \"sandpost\"",
            "7069ddf8b3966a4fa1ecba7aeb04a1b387562d78ce6d302a2947ccfd2c9a7bd7",
        ),
        (
            "from.domain == \"EXAMPLE.COM\" and (subject contains \"invoice\" or not has_attachments == true)",
            "0bf6f35c46b0d989b35e54a93ee4c2447f3f6bf809076018c2233fcd5d0223ee",
        ),
    ] {
        assert_eq!(compile(source).unwrap().fingerprint(), expected, "{source}");
    }
}
