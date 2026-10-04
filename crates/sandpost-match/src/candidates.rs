//! Safe exact-key covers for indexed candidate selection and inherited restrictions.
use sandpost_query::{Expression, Field, Operator, Value};
use std::collections::BTreeSet;

pub(crate) type CandidateAnchor = (Field, Value);

/// Identify fields whose exact positive comparisons support safe candidate lookup.
fn indexable(field: &Field) -> bool {
    matches!(
        field,
        Field::EnvelopeFromAddress
            | Field::EnvelopeFromDomain
            | Field::EnvelopeToAddress
            | Field::EnvelopeToDomain
            | Field::FromAddress
            | Field::FromDomain
            | Field::ToAddress
            | Field::ToDomain
            | Field::CarbonCopyAddress
            | Field::CarbonCopyDomain
            | Field::Header(_)
            | Field::MessageIdentifier
    )
}

/// A cover is a set of exact keys, at least one of which every matching message
/// must contain. None means no safe index anchor. An empty set means impossible.
pub(crate) fn anchor_cover(expression: &Expression) -> Option<BTreeSet<CandidateAnchor>> {
    match expression {
        Expression::False => Some(BTreeSet::new()),
        Expression::Predicate(predicate)
            if predicate.operator == Operator::Equal && indexable(&predicate.field) =>
        {
            Some(BTreeSet::from([(
                predicate.field.clone(),
                predicate.value.clone(),
            )]))
        }
        Expression::And(children) => children
            .iter()
            .map(anchor_cover)
            .fold(None, intersect_cover),
        Expression::Or(children) => {
            let mut keys = BTreeSet::new();
            for child in children {
                keys.extend(anchor_cover(child)?);
            }
            Some(keys)
        }
        _ => None,
    }
}

/// Choose the smaller sufficient conjunction cover; an impossible empty cover wins.
pub(crate) fn intersect_cover(
    first_cover: Option<BTreeSet<CandidateAnchor>>,
    second_cover: Option<BTreeSet<CandidateAnchor>>,
) -> Option<BTreeSet<CandidateAnchor>> {
    match (first_cover, second_cover) {
        (Some(first_cover), Some(second_cover)) => {
            Some(if first_cover.len() <= second_cover.len() {
                first_cover
            } else {
                second_cover
            })
        }
        (Some(keys), None) | (None, Some(keys)) => Some(keys),
        (None, None) => None,
    }
}
