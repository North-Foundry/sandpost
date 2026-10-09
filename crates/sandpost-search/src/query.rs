//! Search request types and semantics-preserving Tantivy candidate planning.

use crate::{keys, schema::SearchFields};
use sandpost_core::{MessageIdentifier, MessageSequence};
use sandpost_query::{Expression, Field as QueryField, Operator, Predicate, Value};
use tantivy::{
    Term,
    query::{AllQuery, BooleanQuery, EmptyQuery, Occur, Query, TermQuery},
    schema::IndexRecordOption,
};

/// Maximum result batch returned by a search request.
pub const MAXIMUM_SEARCH_BATCH_SIZE: usize = 256;

/// Complete search request, including centralized authorization and keyset pagination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageQuery {
    /// `None` allows all mail; otherwise only messages whose facts satisfy the expression.
    ///
    /// `Some(Expression::False)` denies everything without consulting the index.
    pub authorization: Option<Expression>,
    /// Additional caller-selected filter applied after authorization.
    pub filter: Expression,
    /// Optional direct public message identifier restriction.
    pub message_identifier: Option<MessageIdentifier>,
    /// Exclusive upper sequence bound for newest-first keyset pagination.
    pub before: Option<MessageSequence>,
    /// Requested number of IDs; values above [`MAXIMUM_SEARCH_BATCH_SIZE`] are capped.
    pub limit: usize,
}

/// Create a safe superset query; stored facts always decide the final result.
pub(super) fn candidate_query(expression: &Expression, fields: SearchFields) -> Box<dyn Query> {
    match expression {
        Expression::True => Box::new(AllQuery),
        Expression::False => Box::new(EmptyQuery),
        Expression::Predicate(predicate) => predicate_candidate(predicate, fields),
        Expression::Not(_) => Box::new(AllQuery),
        Expression::And(children) if children.is_empty() => Box::new(AllQuery),
        Expression::And(children) => Box::new(BooleanQuery::new(
            children
                .iter()
                .map(|child| (Occur::Must, candidate_query(child, fields)))
                .collect(),
        )),
        Expression::Or(children) if children.is_empty() => Box::new(EmptyQuery),
        Expression::Or(children) => {
            if children
                .iter()
                .any(|child| matches!(child, Expression::True | Expression::Not(_)))
            {
                Box::new(AllQuery)
            } else {
                Box::new(BooleanQuery::new(
                    children
                        .iter()
                        .map(|child| (Occur::Should, candidate_query(child, fields)))
                        .collect(),
                ))
            }
        }
    }
}

/// Report whether candidate hits need evaluation against the canonical expression.
///
/// Only constants, compositions of exact expressions, numeric comparisons, and attachment
/// presence comparisons are represented exactly by indexed fields.
pub(crate) fn requires_verification(expression: &Expression) -> bool {
    match expression {
        Expression::True | Expression::False => false,
        Expression::And(children) | Expression::Or(children) => {
            children.iter().any(requires_verification)
        }
        Expression::Predicate(predicate) => match (&predicate.field, &predicate.value) {
            (
                QueryField::ReceivedAt | QueryField::Size | QueryField::AttachmentCount,
                Value::Number(_),
            ) => !matches!(
                predicate.operator,
                Operator::Equal
                    | Operator::NotEqual
                    | Operator::GreaterThan
                    | Operator::GreaterThanOrEqual
                    | Operator::LessThan
                    | Operator::LessThanOrEqual
            ),
            (QueryField::HasAttachments, Value::Boolean(_)) => {
                !matches!(predicate.operator, Operator::Equal | Operator::NotEqual)
            }
            _ => true,
        },
        Expression::Not(_) => true,
    }
}

/// Plan one predicate without excluding any expression match.
fn predicate_candidate(predicate: &Predicate, fields: SearchFields) -> Box<dyn Query> {
    let field_key = keys::field_key(&predicate.field);
    let fold_case = keys::folds_ascii_case(&predicate.field);
    let substring_field = matches!(
        predicate.field,
        QueryField::Text | QueryField::MarkupBody | QueryField::Content
    );
    match (&predicate.value, predicate.operator) {
        (Value::String(value), Operator::Equal) if substring_field => {
            substring_candidate(&field_key, value, fold_case, fields)
        }
        (Value::String(value), Operator::Equal) => term_query(
            fields.exact_values,
            &keys::exact_term(&field_key, value, fold_case),
        ),
        (Value::Boolean(value), Operator::Equal | Operator::NotEqual)
            if matches!(predicate.field, QueryField::HasAttachments) =>
        {
            let has_attachments = *value ^ (predicate.operator == Operator::NotEqual);
            if has_attachments {
                unsigned_range(fields.attachment_count, 0, Operator::GreaterThan)
            } else {
                Box::new(TermQuery::new(
                    Term::from_field_u64(fields.attachment_count, 0),
                    IndexRecordOption::Basic,
                ))
            }
        }
        (Value::Boolean(value), Operator::Equal) => term_query(
            fields.exact_values,
            &keys::exact_term(&field_key, &value.to_string(), fold_case),
        ),
        (Value::String(_), Operator::NotEqual) => {
            term_query(fields.exact_values, &keys::presence_term(&field_key))
        }
        (Value::String(value), Operator::Contains) => {
            substring_candidate(&field_key, value, fold_case, fields)
        }
        (Value::String(value), Operator::StartsWith) => {
            edge_candidate(&field_key, value, fold_case, true, fields)
        }
        (Value::String(value), Operator::EndsWith) => {
            edge_candidate(&field_key, value, fold_case, false, fields)
        }
        (Value::String(value), Operator::Matches) => {
            glob_candidate(&field_key, value, fold_case, substring_field, fields)
        }
        (Value::Number(value), operator) => {
            numeric_candidate(&predicate.field, *value, operator, fields)
        }
        _ => Box::new(AllQuery),
    }
}

/// Plan ordered scalar numeric predicates with their indexed numeric field.
fn numeric_candidate(
    field: &QueryField,
    value: i64,
    operator: Operator,
    fields: SearchFields,
) -> Box<dyn Query> {
    if matches!(operator, Operator::Equal) {
        return match field {
            QueryField::ReceivedAt => Box::new(TermQuery::new(
                Term::from_field_i64(fields.received_at, value),
                IndexRecordOption::Basic,
            )),
            QueryField::Size | QueryField::AttachmentCount if value >= 0 => {
                Box::new(TermQuery::new(
                    Term::from_field_u64(
                        if matches!(field, QueryField::Size) {
                            fields.size
                        } else {
                            fields.attachment_count
                        },
                        value as u64,
                    ),
                    IndexRecordOption::Basic,
                ))
            }
            QueryField::Size | QueryField::AttachmentCount => Box::new(EmptyQuery),
            _ => Box::new(AllQuery),
        };
    }
    if !matches!(
        operator,
        Operator::NotEqual
            | Operator::GreaterThan
            | Operator::GreaterThanOrEqual
            | Operator::LessThan
            | Operator::LessThanOrEqual
    ) {
        return Box::new(AllQuery);
    }
    if matches!(field, QueryField::ReceivedAt) {
        return signed_range(fields.received_at, value, operator);
    }
    if matches!(field, QueryField::Size | QueryField::AttachmentCount) {
        if value < 0 {
            return if matches!(
                operator,
                Operator::LessThan | Operator::LessThanOrEqual | Operator::Equal
            ) {
                Box::new(EmptyQuery)
            } else {
                Box::new(AllQuery)
            };
        }
        return unsigned_range(
            if matches!(field, QueryField::Size) {
                fields.size
            } else {
                fields.attachment_count
            },
            value as u64,
            operator,
        );
    }
    Box::new(AllQuery)
}

/// Build the signed integer range for one scalar field.
fn signed_range(field: tantivy::schema::Field, value: i64, operator: Operator) -> Box<dyn Query> {
    use Operator::{GreaterThan, GreaterThanOrEqual, LessThan, LessThanOrEqual, NotEqual};
    use std::ops::Bound::{Excluded, Included, Unbounded};
    match operator {
        GreaterThan => range_query(Excluded(Term::from_field_i64(field, value)), Unbounded),
        GreaterThanOrEqual => range_query(Included(Term::from_field_i64(field, value)), Unbounded),
        LessThan => range_query(Unbounded, Excluded(Term::from_field_i64(field, value))),
        LessThanOrEqual => range_query(Unbounded, Included(Term::from_field_i64(field, value))),
        NotEqual => Box::new(BooleanQuery::new(vec![
            (
                Occur::Should,
                range_query(Unbounded, Excluded(Term::from_field_i64(field, value))),
            ),
            (
                Occur::Should,
                range_query(Excluded(Term::from_field_i64(field, value)), Unbounded),
            ),
        ])),
        _ => Box::new(AllQuery),
    }
}

/// Build an unsigned integer range for one scalar field.
fn unsigned_range(field: tantivy::schema::Field, value: u64, operator: Operator) -> Box<dyn Query> {
    use Operator::{GreaterThan, GreaterThanOrEqual, LessThan, LessThanOrEqual, NotEqual};
    use std::ops::Bound::{Excluded, Included, Unbounded};
    match operator {
        GreaterThan => range_query(Excluded(Term::from_field_u64(field, value)), Unbounded),
        GreaterThanOrEqual => range_query(Included(Term::from_field_u64(field, value)), Unbounded),
        LessThan => range_query(Unbounded, Excluded(Term::from_field_u64(field, value))),
        LessThanOrEqual => range_query(Unbounded, Included(Term::from_field_u64(field, value))),
        NotEqual => Box::new(BooleanQuery::new(vec![
            (
                Occur::Should,
                range_query(Unbounded, Excluded(Term::from_field_u64(field, value))),
            ),
            (
                Occur::Should,
                range_query(Excluded(Term::from_field_u64(field, value)), Unbounded),
            ),
        ])),
        _ => Box::new(AllQuery),
    }
}

/// Construct a Tantivy integer range query.
fn range_query(lower: std::ops::Bound<Term>, upper: std::ops::Bound<Term>) -> Box<dyn Query> {
    Box::new(tantivy::query::RangeQuery::new(lower, upper))
}

/// Narrow substring predicates using all distinct trigrams, or field presence for short strings.
fn substring_candidate(
    field_key: &str,
    value: &str,
    fold_case: bool,
    fields: SearchFields,
) -> Box<dyn Query> {
    let trigrams = keys::distinct_trigrams(value, fold_case);
    if trigrams.is_empty() {
        return term_query(fields.exact_values, &keys::presence_term(field_key));
    }
    Box::new(BooleanQuery::new(
        trigrams
            .iter()
            .map(|trigram| {
                let trigram: String = trigram.iter().copied().collect();
                (
                    Occur::Must,
                    term_query(fields.trigrams, &keys::trigram_term(field_key, &trigram)),
                )
            })
            .collect(),
    ))
}

/// Narrow starts-with or ends-with using its boundary trigram when available.
fn edge_candidate(
    field_key: &str,
    value: &str,
    fold_case: bool,
    prefix: bool,
    fields: SearchFields,
) -> Box<dyn Query> {
    let fold = |character: char| {
        if fold_case {
            character.to_ascii_lowercase()
        } else {
            character
        }
    };
    let boundary: String = if prefix {
        value.chars().map(fold).take(3).collect()
    } else {
        let mut characters: Vec<char> = value.chars().rev().map(fold).take(3).collect();
        characters.reverse();
        characters.into_iter().collect()
    };
    if boundary.chars().count() < 3 {
        return term_query(fields.exact_values, &keys::presence_term(field_key));
    }
    term_query(fields.trigrams, &keys::trigram_term(field_key, &boundary))
}

/// Extract a sufficiently long literal run from a glob pattern as a safe candidate anchor.
fn glob_candidate(
    field_key: &str,
    pattern: &str,
    fold_case: bool,
    substring_field: bool,
    fields: SearchFields,
) -> Box<dyn Query> {
    if !pattern.contains(['*', '?']) {
        if substring_field {
            return substring_candidate(field_key, pattern, fold_case, fields);
        }
        return term_query(
            fields.exact_values,
            &keys::exact_term(field_key, pattern, fold_case),
        );
    }
    let longest = pattern
        .split(['*', '?'])
        .max_by_key(|part| part.chars().count())
        .unwrap_or_default();
    if longest.chars().count() >= 3 {
        substring_candidate(field_key, longest, fold_case, fields)
    } else {
        term_query(fields.exact_values, &keys::presence_term(field_key))
    }
}

/// Build an indexed exact-term query.
fn term_query(field: tantivy::schema::Field, value: &str) -> Box<dyn Query> {
    if value.len() > tantivy::tokenizer::MAX_TOKEN_LEN {
        return Box::new(AllQuery);
    }
    Box::new(TermQuery::new(
        Term::from_field_text(field, value),
        IndexRecordOption::Basic,
    ))
}

#[cfg(test)]
mod tests {
    use super::requires_verification;
    use sandpost_query::{Expression, Field, Operator, Predicate, Value};

    /// Build a predicate expression for verification classification tests.
    fn predicate(field: Field, operator: Operator, value: Value) -> Expression {
        Expression::Predicate(Predicate {
            field,
            operator,
            value,
        })
    }

    /// Confirm indexed numeric ranges and attachment equality need no reevaluation.
    #[test]
    fn supported_numeric_and_attachment_predicates_are_exact() {
        let numeric_fields = [Field::ReceivedAt, Field::Size, Field::AttachmentCount];
        let numeric_operators = [
            Operator::Equal,
            Operator::NotEqual,
            Operator::GreaterThan,
            Operator::GreaterThanOrEqual,
            Operator::LessThan,
            Operator::LessThanOrEqual,
        ];
        for field in numeric_fields {
            for operator in numeric_operators {
                assert!(!requires_verification(&predicate(
                    field.clone(),
                    operator,
                    Value::Number(12),
                )));
            }
        }
        for operator in [Operator::Equal, Operator::NotEqual] {
            for value in [false, true] {
                assert!(!requires_verification(&predicate(
                    Field::HasAttachments,
                    operator,
                    Value::Boolean(value),
                )));
            }
        }
    }

    /// Keep unsupported operators and ill-typed predicates on the exact evaluator path.
    #[test]
    fn unsupported_and_mistyped_predicates_require_verification() {
        for expression in [
            predicate(Field::Size, Operator::Contains, Value::Number(12)),
            predicate(Field::ReceivedAt, Operator::Equal, Value::Boolean(true)),
            predicate(
                Field::HasAttachments,
                Operator::Contains,
                Value::Boolean(false),
            ),
            predicate(Field::Subject, Operator::Equal, Value::Number(12)),
        ] {
            assert!(requires_verification(&expression));
        }
    }

    /// Confirm exactness composes through AND/OR, while NOT stays conservative.
    #[test]
    fn boolean_compositions_preserve_verification_requirements() {
        let exact = predicate(Field::ReceivedAt, Operator::GreaterThan, Value::Number(0));
        let inexact = predicate(
            Field::Subject,
            Operator::Contains,
            Value::String("needle".into()),
        );
        assert!(!requires_verification(&Expression::True));
        assert!(!requires_verification(&Expression::False));
        assert!(!requires_verification(&Expression::And(vec![
            exact.clone(),
            Expression::True,
        ])));
        assert!(!requires_verification(&Expression::Or(vec![exact.clone()])));
        assert!(requires_verification(&Expression::And(vec![
            exact.clone(),
            inexact.clone(),
        ])));
        assert!(requires_verification(&Expression::Or(vec![
            exact.clone(),
            inexact
        ])));
        assert!(requires_verification(&Expression::Not(Box::new(exact))));
    }
}
