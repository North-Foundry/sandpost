//! Search request types and semantics-preserving Tantivy candidate planning.

use crate::{index::SearchFields, keys};
use sandpost_core::{EndpointIdentifier, MessageIdentifier, MessageSequence};
use sandpost_query::{Expression, Field as QueryField, Operator, Predicate, Value};
use tantivy::{
    Term,
    query::{AllQuery, BooleanQuery, Occur, Query, TermQuery},
    schema::IndexRecordOption,
};

/// Maximum result batch returned by a search request.
pub const MAXIMUM_SEARCH_BATCH_SIZE: usize = 256;

/// One endpoint-bound expression in an OR authorization set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointQuery {
    /// Endpoint whose documents this expression may authorize.
    pub endpoint_identifier: EndpointIdentifier,
    /// Query evaluated against the message facts after endpoint restriction.
    pub expression: Expression,
}

/// Complete search request, including centralized authorization and keyset pagination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageQuery {
    /// `None` allows global access; `Some([])` denies all; otherwise clauses are ORed.
    pub authorization: Option<Vec<EndpointQuery>>,
    /// Additional caller-selected filter applied after authorization.
    pub filter: Expression,
    /// Optional endpoint restriction applied to authorized results.
    pub endpoint_identifier: Option<EndpointIdentifier>,
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
        Expression::False => term_query(fields.marker, keys::NEVER_MARKER),
        Expression::Predicate(predicate) => predicate_candidate(predicate, fields),
        Expression::Not(_) => Box::new(AllQuery),
        Expression::And(children) if children.is_empty() => Box::new(AllQuery),
        Expression::And(children) => Box::new(BooleanQuery::new(
            children
                .iter()
                .map(|child| (Occur::Must, candidate_query(child, fields)))
                .collect(),
        )),
        Expression::Or(children) if children.is_empty() => {
            term_query(fields.marker, keys::NEVER_MARKER)
        }
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
            QueryField::Size | QueryField::AttachmentCount => {
                term_query(fields.marker, keys::NEVER_MARKER)
            }
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
        return signed_range(fields.received_at, value, operator, fields.marker);
    }
    if matches!(field, QueryField::Size | QueryField::AttachmentCount) {
        if value < 0 {
            return if matches!(
                operator,
                Operator::LessThan | Operator::LessThanOrEqual | Operator::Equal
            ) {
                term_query(fields.marker, keys::NEVER_MARKER)
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
            fields.marker,
        );
    }
    Box::new(AllQuery)
}

/// Build the signed integer range for one scalar field.
fn signed_range(
    field: tantivy::schema::Field,
    value: i64,
    operator: Operator,
    marker: tantivy::schema::Field,
) -> Box<dyn Query> {
    use Operator::{GreaterThan, GreaterThanOrEqual, LessThan, LessThanOrEqual, NotEqual};
    use std::ops::Bound::{Excluded, Included, Unbounded};
    match operator {
        GreaterThan => signed_range_query(Excluded(Term::from_field_i64(field, value)), Unbounded),
        GreaterThanOrEqual => {
            signed_range_query(Included(Term::from_field_i64(field, value)), Unbounded)
        }
        LessThan => signed_range_query(Unbounded, Excluded(Term::from_field_i64(field, value))),
        LessThanOrEqual => {
            signed_range_query(Unbounded, Included(Term::from_field_i64(field, value)))
        }
        NotEqual => Box::new(BooleanQuery::new(vec![
            (
                Occur::Should,
                signed_range_query(Unbounded, Excluded(Term::from_field_i64(field, value))),
            ),
            (
                Occur::Should,
                signed_range_query(Excluded(Term::from_field_i64(field, value)), Unbounded),
            ),
        ])),
        _ => term_query(marker, keys::NEVER_MARKER),
    }
}

/// Build an unsigned integer range for one scalar field.
fn unsigned_range(
    field: tantivy::schema::Field,
    value: u64,
    operator: Operator,
    marker: tantivy::schema::Field,
) -> Box<dyn Query> {
    use Operator::{GreaterThan, GreaterThanOrEqual, LessThan, LessThanOrEqual, NotEqual};
    use std::ops::Bound::{Excluded, Included, Unbounded};
    match operator {
        GreaterThan => {
            unsigned_range_query(Excluded(Term::from_field_u64(field, value)), Unbounded)
        }
        GreaterThanOrEqual => {
            unsigned_range_query(Included(Term::from_field_u64(field, value)), Unbounded)
        }
        LessThan => unsigned_range_query(Unbounded, Excluded(Term::from_field_u64(field, value))),
        LessThanOrEqual => {
            unsigned_range_query(Unbounded, Included(Term::from_field_u64(field, value)))
        }
        NotEqual => Box::new(BooleanQuery::new(vec![
            (
                Occur::Should,
                unsigned_range_query(Unbounded, Excluded(Term::from_field_u64(field, value))),
            ),
            (
                Occur::Should,
                unsigned_range_query(Excluded(Term::from_field_u64(field, value)), Unbounded),
            ),
        ])),
        _ => term_query(marker, keys::NEVER_MARKER),
    }
}

/// Construct a Tantivy signed range query.
fn signed_range_query(
    lower: std::ops::Bound<Term>,
    upper: std::ops::Bound<Term>,
) -> Box<dyn Query> {
    Box::new(tantivy::query::RangeQuery::new(lower, upper))
}

/// Construct a Tantivy unsigned range query.
fn unsigned_range_query(
    lower: std::ops::Bound<Term>,
    upper: std::ops::Bound<Term>,
) -> Box<dyn Query> {
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
    let normalized = keys::normalize_string(value, fold_case);
    let characters: Vec<char> = normalized.chars().collect();
    if characters.len() < 3 {
        return term_query(fields.exact_values, &keys::presence_term(field_key));
    }
    let boundary: String = if prefix {
        characters[..3].iter().collect()
    } else {
        characters[characters.len() - 3..].iter().collect()
    };
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
    Box::new(TermQuery::new(
        Term::from_field_text(field, value),
        IndexRecordOption::Basic,
    ))
}
