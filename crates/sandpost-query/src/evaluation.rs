//! Evaluation against message facts, including collection ANY semantics.
use crate::{
    expression::{Expression, Field, Operator, Predicate, Value},
    glob::glob_matches,
};
use sandpost_core::MessageFacts;
use std::cmp::Ordering;

impl Expression {
    /// Evaluate boolean structure and its predicates against message facts.
    pub fn evaluate(&self, facts: &MessageFacts) -> bool {
        match self {
            Self::True => true,
            Self::False => false,
            Self::Predicate(predicate) => predicate.evaluate(facts),
            Self::Not(operand) => !operand.evaluate(facts),
            Self::And(children) => children.iter().all(|child| child.evaluate(facts)),
            Self::Or(children) => children.iter().any(|child| child.evaluate(facts)),
        }
    }
}

impl Predicate {
    /// Relative field-based planning hint, not a runtime or complexity guarantee.
    pub fn cost(&self) -> u8 {
        match self.field {
            Field::MessageIdentifier
            | Field::ReceivedAt
            | Field::Size
            | Field::AttachmentCount
            | Field::HasAttachments => 1,
            Field::Content => 7,
            Field::EnvelopeFromAddress
            | Field::EnvelopeFromDomain
            | Field::FromAddress
            | Field::FromDomain => 2,
            Field::Header(_) => 4,
            _ => 6,
        }
    }

    /// Returns all typed values for this field. List fields intentionally retain all entries.
    pub fn field_values(&self, facts: &MessageFacts) -> Vec<Value> {
        self.field.values(facts)
    }

    /// ANY semantics apply to every field. In particular, `!=` is any value unequal to the RHS.
    pub fn evaluate(&self, facts: &MessageFacts) -> bool {
        self.field.any_value_matches(facts, |actual_value| {
            compare_borrowed_value(actual_value, &self.value, self.operator, &self.field)
        })
    }
}

impl Field {
    /// Owned values for index projections. Unsigned numeric facts above `i64::MAX`
    /// saturate here; [`Expression::evaluate`] compares those facts exactly without copying.
    pub fn values(&self, facts: &MessageFacts) -> Vec<Value> {
        let owned_strings = |values: Vec<String>| values.into_iter().map(Value::String).collect();
        let mailbox_values = |mailboxes: &[sandpost_core::Mailbox], use_domain: bool| {
            mailboxes
                .iter()
                .map(|mailbox| {
                    Value::String(if use_domain {
                        mailbox.domain.clone()
                    } else {
                        mailbox.address.clone()
                    })
                })
                .collect()
        };
        match self {
            Self::EnvelopeFromAddress => facts
                .envelope_from
                .iter()
                .map(|mailbox| Value::String(mailbox.address.clone()))
                .collect(),
            Self::EnvelopeFromDomain => facts
                .envelope_from
                .iter()
                .map(|mailbox| Value::String(mailbox.domain.clone()))
                .collect(),
            Self::EnvelopeToAddress => mailbox_values(&facts.envelope_to, false),
            Self::EnvelopeToDomain => mailbox_values(&facts.envelope_to, true),
            Self::FromAddress => mailbox_values(&facts.from, false),
            Self::FromDomain => mailbox_values(&facts.from, true),
            Self::ToAddress => mailbox_values(&facts.to, false),
            Self::ToDomain => mailbox_values(&facts.to, true),
            Self::CarbonCopyAddress => mailbox_values(&facts.carbon_copy, false),
            Self::CarbonCopyDomain => mailbox_values(&facts.carbon_copy, true),
            Self::Subject => vec![Value::String(facts.subject.clone())],
            Self::Text => vec![Value::String(facts.text.clone())],
            Self::MarkupBody => vec![Value::String(facts.markup_body.clone())],
            Self::Content => content_strings(facts)
                .into_iter()
                .map(|value| Value::String(value.to_ascii_lowercase()))
                .collect(),
            Self::MessageIdentifier => {
                owned_strings(facts.message_identifier.iter().cloned().collect())
            }
            Self::ReceivedAt => vec![Value::Number(facts.received_at)],
            Self::Size => vec![Value::Number(i64::try_from(facts.size).unwrap_or(i64::MAX))],
            Self::AttachmentCount => vec![Value::Number(
                i64::try_from(facts.attachment_count).unwrap_or(i64::MAX),
            )],
            Self::HasAttachments => vec![Value::Boolean(facts.attachment_count > 0)],
            Self::Header(name) => {
                owned_strings(facts.headers.get(name).cloned().unwrap_or_default())
            }
        }
    }

    /// Apply a predicate to each present field value and stop at the first match.
    fn any_value_matches(
        &self,
        facts: &MessageFacts,
        mut matches_value: impl FnMut(&BorrowedValue<'_>) -> bool,
    ) -> bool {
        let mailbox_values_match =
            |mailboxes: &[sandpost_core::Mailbox],
             use_domain: bool,
             matches_value: &mut dyn FnMut(&BorrowedValue<'_>) -> bool| {
                mailboxes.iter().any(|mailbox| {
                    matches_value(&if use_domain {
                        BorrowedValue::String(&mailbox.domain)
                    } else {
                        BorrowedValue::String(&mailbox.address)
                    })
                })
            };
        match self {
            Self::EnvelopeFromAddress => facts
                .envelope_from
                .iter()
                .any(|mailbox| matches_value(&BorrowedValue::String(&mailbox.address))),
            Self::EnvelopeFromDomain => facts
                .envelope_from
                .iter()
                .any(|mailbox| matches_value(&BorrowedValue::String(&mailbox.domain))),
            Self::EnvelopeToAddress => {
                mailbox_values_match(&facts.envelope_to, false, &mut matches_value)
            }
            Self::EnvelopeToDomain => {
                mailbox_values_match(&facts.envelope_to, true, &mut matches_value)
            }
            Self::FromAddress => mailbox_values_match(&facts.from, false, &mut matches_value),
            Self::FromDomain => mailbox_values_match(&facts.from, true, &mut matches_value),
            Self::ToAddress => mailbox_values_match(&facts.to, false, &mut matches_value),
            Self::ToDomain => mailbox_values_match(&facts.to, true, &mut matches_value),
            Self::CarbonCopyAddress => {
                mailbox_values_match(&facts.carbon_copy, false, &mut matches_value)
            }
            Self::CarbonCopyDomain => {
                mailbox_values_match(&facts.carbon_copy, true, &mut matches_value)
            }
            Self::Subject => matches_value(&BorrowedValue::String(&facts.subject)),
            Self::Text => matches_value(&BorrowedValue::String(&facts.text)),
            Self::MarkupBody => matches_value(&BorrowedValue::String(&facts.markup_body)),
            Self::Content => content_strings(facts)
                .into_iter()
                .any(|value| matches_value(&BorrowedValue::String(value))),
            Self::MessageIdentifier => facts
                .message_identifier
                .as_deref()
                .is_some_and(|string| matches_value(&BorrowedValue::String(string))),
            Self::ReceivedAt => matches_value(&BorrowedValue::Number(facts.received_at)),
            Self::Size => matches_value(&BorrowedValue::Unsigned(facts.size)),
            Self::AttachmentCount => {
                matches_value(&BorrowedValue::Unsigned(facts.attachment_count))
            }
            Self::HasAttachments => {
                matches_value(&BorrowedValue::Boolean(facts.attachment_count > 0))
            }
            Self::Header(name) => facts.headers.get(name).is_some_and(|values| {
                values
                    .iter()
                    .any(|string| matches_value(&BorrowedValue::String(string)))
            }),
        }
    }
}

enum BorrowedValue<'message_value> {
    String(&'message_value str),
    Number(i64),
    Unsigned(u64),
    Boolean(bool),
}

/// Compare a borrowed fact value with a typed query literal using the field's case rules.
fn compare_borrowed_value(
    actual_value: &BorrowedValue<'_>,
    expected_value: &Value,
    operator: Operator,
    field: &Field,
) -> bool {
    let ordering = match (actual_value, expected_value) {
        (BorrowedValue::String(actual_string), Value::String(expected_string))
            if field.uses_ascii_case_insensitive_ordering()
                || (field.is_mailbox_field()
                    && matches!(operator, Operator::Equal | Operator::NotEqual)) =>
        {
            Some(compare_ascii_case_insensitive(
                actual_string,
                expected_string,
            ))
        }
        (BorrowedValue::String(actual_string), Value::String(expected_string)) => {
            Some(actual_string.cmp(&expected_string.as_str()))
        }
        (BorrowedValue::Number(actual_number), Value::Number(expected_number)) => {
            Some(actual_number.cmp(expected_number))
        }
        (BorrowedValue::Unsigned(actual_number), Value::Number(expected_number)) => {
            Some(if *expected_number < 0 {
                Ordering::Greater
            } else {
                actual_number.cmp(&(*expected_number as u64))
            })
        }
        (BorrowedValue::Boolean(actual_boolean), Value::Boolean(expected_boolean)) => {
            Some(actual_boolean.cmp(expected_boolean))
        }
        _ => None,
    };
    match operator {
        Operator::Equal => ordering == Some(Ordering::Equal),
        Operator::NotEqual => ordering.is_some_and(|ordering| ordering != Ordering::Equal),
        Operator::GreaterThan => ordering == Some(Ordering::Greater),
        Operator::GreaterThanOrEqual => ordering.is_some_and(|ordering| ordering != Ordering::Less),
        Operator::LessThan => ordering == Some(Ordering::Less),
        Operator::LessThanOrEqual => ordering.is_some_and(|ordering| ordering != Ordering::Greater),
        Operator::Contains | Operator::StartsWith | Operator::EndsWith | Operator::Matches => {
            let (BorrowedValue::String(text), Value::String(pattern)) =
                (actual_value, expected_value)
            else {
                return false;
            };
            compare_strings(text, pattern, operator, field.is_ascii_case_insensitive())
        }
    }
}

/// Return all literal-search values, including header names, without allocating strings.
fn content_strings(facts: &MessageFacts) -> Vec<&str> {
    let mut values = vec![
        facts.subject.as_str(),
        facts.text.as_str(),
        facts.markup_body.as_str(),
    ];
    values.extend(
        facts
            .envelope_from
            .iter()
            .map(|mailbox| mailbox.address.as_str()),
    );
    values.extend(
        facts
            .envelope_to
            .iter()
            .map(|mailbox| mailbox.address.as_str()),
    );
    values.extend(facts.from.iter().map(|mailbox| mailbox.address.as_str()));
    values.extend(facts.to.iter().map(|mailbox| mailbox.address.as_str()));
    values.extend(
        facts
            .carbon_copy
            .iter()
            .map(|mailbox| mailbox.address.as_str()),
    );
    if let Some(identifier) = facts.message_identifier.as_deref() {
        values.push(identifier);
    }
    for (name, header_values) in &facts.headers {
        values.push(name);
        values.extend(header_values.iter().map(String::as_str));
    }
    values
}

/// Compare UTF-8 strings lexicographically after folding ASCII letters only.
fn compare_ascii_case_insensitive(first: &str, second: &str) -> Ordering {
    first
        .bytes()
        .map(|byte| byte.to_ascii_lowercase())
        .cmp(second.bytes().map(|byte| byte.to_ascii_lowercase()))
}

/// Apply a string operator, using ASCII-insensitive matching only for mailbox fields.
fn compare_strings(
    text: &str,
    pattern: &str,
    operator: Operator,
    ignore_basic_latin_case: bool,
) -> bool {
    match operator {
        Operator::Contains if ignore_basic_latin_case => {
            contains_basic_latin_case_insensitive(text, pattern)
        }
        Operator::Contains => text.contains(pattern),
        Operator::StartsWith if ignore_basic_latin_case => {
            starts_basic_latin_case_insensitive(text, pattern)
        }
        Operator::StartsWith => text.starts_with(pattern),
        Operator::EndsWith if ignore_basic_latin_case => {
            ends_basic_latin_case_insensitive(text, pattern)
        }
        Operator::EndsWith => text.ends_with(pattern),
        Operator::Matches => glob_matches(text, pattern, ignore_basic_latin_case),
        _ => unreachable!("comparison operators are handled before string matching"),
    }
}

/// Check whether text contains a byte-aligned ASCII-insensitive substring.
fn contains_basic_latin_case_insensitive(text: &str, search_string: &str) -> bool {
    search_string.is_empty()
        || text
            .as_bytes()
            .windows(search_string.len())
            .any(|window| window.eq_ignore_ascii_case(search_string.as_bytes()))
}

/// Check whether text begins with an ASCII-insensitive prefix.
fn starts_basic_latin_case_insensitive(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|slice| slice.eq_ignore_ascii_case(prefix))
}

/// Check whether text ends with an ASCII-insensitive suffix.
fn ends_basic_latin_case_insensitive(text: &str, suffix: &str) -> bool {
    text.len()
        .checked_sub(suffix.len())
        .and_then(|byte_index| text.get(byte_index..))
        .is_some_and(|slice| slice.eq_ignore_ascii_case(suffix))
}
