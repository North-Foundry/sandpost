//! Canonical forms and their versioned SHA-256 identity.
use crate::expression::{Expression, Field, Operator, Value};
use sha2::{Digest, Sha256};

impl Expression {
    /// Normalize boolean structure into a stable form used for evaluation and fingerprints.
    pub fn canonicalize(self) -> Self {
        match self {
            Self::Not(operand) => match operand.canonicalize() {
                Self::True => Self::False,
                Self::False => Self::True,
                Self::Not(inner) => *inner,
                other => Self::Not(Box::new(other)),
            },
            Self::And(children) => normalize_boolean_children(children, true),
            Self::Or(children) => normalize_boolean_children(children, false),
            other => other,
        }
    }
}

/// Flatten, simplify, sort, and deduplicate the children of an AND or OR expression.
fn normalize_boolean_children(expressions: Vec<Expression>, is_conjunction: bool) -> Expression {
    let mut children = Vec::new();
    for expression in expressions.into_iter().map(Expression::canonicalize) {
        match (is_conjunction, expression) {
            (true, Expression::False) | (false, Expression::True) => {
                return if is_conjunction {
                    Expression::False
                } else {
                    Expression::True
                };
            }
            (true, Expression::True) | (false, Expression::False) => {}
            (true, Expression::And(nested)) | (false, Expression::Or(nested)) => {
                children.extend(nested)
            }
            (_, expression) => children.push(expression),
        }
    }
    children.sort();
    children.dedup();
    if children.iter().any(|expression| {
        children
            .binary_search(&negate_expression(expression.clone()))
            .is_ok()
    }) {
        return if is_conjunction {
            Expression::False
        } else {
            Expression::True
        };
    }
    match children.len() {
        0 => {
            if is_conjunction {
                Expression::True
            } else {
                Expression::False
            }
        }
        1 => children.pop().unwrap(),
        _ => {
            if is_conjunction {
                Expression::And(children)
            } else {
                Expression::Or(children)
            }
        }
    }
}

/// Return the expression's explicit negation, removing one existing negation when present.
fn negate_expression(expression: Expression) -> Expression {
    match expression {
        Expression::Not(inner) => *inner,
        other => Expression::Not(Box::new(other)),
    }
}

/// Return lowercase SHA-256 for the unchanged versioned canonical encoding.
pub(crate) fn fingerprint(expression: &Expression) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"sandpost-query\0v1\0");
    fingerprint_expression(&mut hasher, expression);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Feed one expression into the v1 tagged, length-delimited fingerprint encoding.
fn fingerprint_expression(hasher: &mut Sha256, expression: &Expression) {
    match expression {
        Expression::True => hasher.update([0]),
        Expression::False => hasher.update([1]),
        Expression::Predicate(predicate) => {
            hasher.update([2]);
            fingerprint_field(hasher, &predicate.field);
            hasher.update([operator_tag(predicate.operator)]);
            match &predicate.value {
                Value::String(string) => {
                    hasher.update([0]);
                    fingerprint_string(hasher, string);
                }
                Value::Number(number) => {
                    hasher.update([1]);
                    hasher.update(number.to_be_bytes());
                }
                Value::Boolean(boolean) => hasher.update([2, u8::from(*boolean)]),
            }
        }
        Expression::Not(operand) => {
            hasher.update([3]);
            fingerprint_expression(hasher, operand);
        }
        Expression::And(children) | Expression::Or(children) => {
            hasher.update([if matches!(expression, Expression::And(_)) {
                4
            } else {
                5
            }]);
            hasher.update((children.len() as u64).to_be_bytes());
            for child in children {
                fingerprint_expression(hasher, child);
            }
        }
    }
}

/// Map an operator to its stable v1 fingerprint tag.
fn operator_tag(operator: Operator) -> u8 {
    match operator {
        Operator::Equal => 0,
        Operator::NotEqual => 1,
        Operator::GreaterThan => 2,
        Operator::GreaterThanOrEqual => 3,
        Operator::LessThan => 4,
        Operator::LessThanOrEqual => 5,
        Operator::Contains => 6,
        Operator::StartsWith => 7,
        Operator::EndsWith => 8,
        Operator::Matches => 9,
    }
}

/// Encode a string as its byte length followed by its UTF-8 bytes.
fn fingerprint_string(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

/// Encode a field using its stable v1 tag and, for headers, its normalized name.
fn fingerprint_field(hasher: &mut Sha256, field: &Field) {
    let tag = match field {
        Field::EnvelopeFromAddress => 0,
        Field::EnvelopeFromDomain => 1,
        Field::EnvelopeToAddress => 2,
        Field::EnvelopeToDomain => 3,
        Field::FromAddress => 4,
        Field::FromDomain => 5,
        Field::ToAddress => 6,
        Field::ToDomain => 7,
        Field::CarbonCopyAddress => 8,
        Field::CarbonCopyDomain => 9,
        Field::Subject => 10,
        Field::Text => 11,
        Field::MarkupBody => 12,
        Field::MessageIdentifier => 13,
        Field::ReceivedAt => 14,
        Field::Size => 15,
        Field::AttachmentCount => 16,
        Field::HasAttachments => 17,
        Field::Header(_) => 18,
        Field::Content => 19,
    };
    hasher.update([tag]);
    if let Field::Header(name) = field {
        fingerprint_string(hasher, name);
    }
}
