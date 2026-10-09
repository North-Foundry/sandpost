//! Shared index keys and term encoding for indexing and candidate planning.
//!
//! The document writer ([crate::index]) and the candidate planner ([crate::query]) must produce
//! byte-identical keys and terms. Both build them from this module so the two cannot drift apart
//! and silently stop matching each other.

use sandpost_query::Field as QueryField;
use std::collections::HashSet;

/// Stable index key for one DSL field.
pub(super) fn field_key(field: &QueryField) -> String {
    match field {
        QueryField::EnvelopeFromAddress => "envelope.from.address".into(),
        QueryField::EnvelopeFromDomain => "envelope.from.domain".into(),
        QueryField::EnvelopeToAddress => "envelope.to.address".into(),
        QueryField::EnvelopeToDomain => "envelope.to.domain".into(),
        QueryField::FromAddress => "from.address".into(),
        QueryField::FromDomain => "from.domain".into(),
        QueryField::ToAddress => "to.address".into(),
        QueryField::ToDomain => "to.domain".into(),
        QueryField::CarbonCopyAddress => "cc.address".into(),
        QueryField::CarbonCopyDomain => "cc.domain".into(),
        QueryField::Subject => "subject".into(),
        QueryField::Text => "text".into(),
        QueryField::MarkupBody => "html".into(),
        QueryField::Content => "content".into(),
        QueryField::MessageIdentifier => "message_id".into(),
        QueryField::ReceivedAt => "received_at".into(),
        QueryField::Size => "size".into(),
        QueryField::AttachmentCount => "attachment_count".into(),
        QueryField::HasAttachments => "has_attachments".into(),
        QueryField::Header(name) => format!("header:{name}"),
    }
}

/// Report whether this field folds ASCII letter case in its string terms.
pub(super) fn folds_ascii_case(field: &QueryField) -> bool {
    field.is_ascii_case_insensitive()
}

/// Fold ASCII case only when the field requires it.
pub(super) fn normalize_string(value: &str, fold_case: bool) -> String {
    if fold_case {
        value.to_ascii_lowercase()
    } else {
        value.to_owned()
    }
}

/// Encode an exact-value term.
pub(super) fn exact_term(field_key: &str, value: &str, fold_case: bool) -> String {
    format!("{field_key}\u{1f}{}", normalize_string(value, fold_case))
}

/// Encode a field-presence term used for collection-safe inequality candidates.
pub(super) fn presence_term(field_key: &str) -> String {
    format!("{field_key}\u{1f}*")
}

/// Encode one substring term for a field.
pub(super) fn trigram_term(field_key: &str, trigram: &str) -> String {
    format!("{field_key}\u{1f}{trigram}")
}

/// Collect the distinct Unicode-scalar trigrams of a value.
///
/// Memory scales with the number of distinct trigrams rather than the value length times
/// per-trigram string overhead.
pub(super) fn distinct_trigrams(value: &str, fold_case: bool) -> HashSet<[char; 3]> {
    let mut characters = value.chars().map(|character| {
        if fold_case {
            character.to_ascii_lowercase()
        } else {
            character
        }
    });
    let Some(first) = characters.next() else {
        return HashSet::new();
    };
    let Some(second) = characters.next() else {
        return HashSet::new();
    };
    let Some(third) = characters.next() else {
        return HashSet::new();
    };
    let mut window = [first, second, third];
    let mut trigrams = HashSet::new();
    trigrams.insert(window);
    for character in characters {
        window.rotate_left(1);
        window[2] = character;
        trigrams.insert(window);
    }
    trigrams
}
