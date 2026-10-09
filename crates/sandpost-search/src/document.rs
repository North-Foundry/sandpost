//! Public index input and private encoding of facts into searchable terms.

use crate::{SearchError, keys, schema::SearchFields};
use sandpost_core::{MessageFacts, MessageIdentifier, MessageSequence};
use sandpost_query::Field as QueryField;
use serde::{Deserialize, Serialize};
use tantivy::TantivyDocument;

/// Searchable facts for one persisted message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedDocument {
    /// Public message identifier used for idempotent replacement and deletion.
    pub identifier: MessageIdentifier,
    /// Unique, monotonically increasing persistence sequence used for pagination.
    pub sequence: MessageSequence,
    /// Persistence revision bound to the message facts for authorization hydration.
    pub revision: u64,
    /// Normalized message facts evaluated by the canonical query interpreter.
    pub facts: MessageFacts,
}

/// Build the stored Tantivy representation and its safe candidate terms.
pub(super) fn tantivy_document(
    document: &IndexedDocument,
    fields: SearchFields,
) -> Result<TantivyDocument, SearchError> {
    let mut indexed = TantivyDocument::default();
    indexed.add_text(fields.identifier, document.identifier.to_string());
    indexed.add_u64(fields.sequence, document.sequence.0);
    indexed.add_u64(fields.revision, document.revision);
    indexed.add_text(fields.facts, serde_json::to_string(&document.facts)?);
    indexed.add_i64(fields.received_at, document.facts.received_at);
    indexed.add_u64(fields.size, document.facts.size);
    indexed.add_u64(fields.attachment_count, document.facts.attachment_count);
    for (field, values) in indexed_string_values(&document.facts) {
        let field_key = keys::field_key(&field);
        let fold_case = keys::folds_ascii_case(&field);
        let body_field = matches!(field, QueryField::Text | QueryField::MarkupBody);
        if !values.is_empty() && field_key.len() + 2 <= tantivy::tokenizer::MAX_TOKEN_LEN {
            indexed.add_text(fields.exact_values, keys::presence_term(&field_key));
        }
        for value in values {
            if !body_field && field_key.len() + 1 + value.len() <= tantivy::tokenizer::MAX_TOKEN_LEN
            {
                indexed.add_text(
                    fields.exact_values,
                    keys::exact_term(&field_key, value, fold_case),
                );
            }
            if field_key.len() + 4 > tantivy::tokenizer::MAX_TOKEN_LEN {
                continue;
            }
            for trigram in keys::distinct_trigrams(value, fold_case) {
                let trigram: String = trigram.iter().copied().collect();
                if field_key.len() + 1 + trigram.len() <= tantivy::tokenizer::MAX_TOKEN_LEN {
                    indexed.add_text(fields.trigrams, keys::trigram_term(&field_key, &trigram));
                }
            }
        }
    }
    let content_key = keys::field_key(&QueryField::Content);
    indexed.add_text(fields.exact_values, keys::presence_term(&content_key));
    let mut content_trigrams = std::collections::HashSet::new();
    for value in indexed_content_values(&document.facts) {
        content_trigrams.extend(keys::distinct_trigrams(value, true));
    }
    for trigram in content_trigrams {
        let trigram: String = trigram.iter().copied().collect();
        indexed.add_text(fields.trigrams, keys::trigram_term(&content_key, &trigram));
    }
    Ok(indexed)
}

/// Return every queryable stored string with its DSL field, without copying message strings.
fn indexed_string_values(facts: &MessageFacts) -> Vec<(QueryField, Vec<&str>)> {
    let mut values = vec![
        (
            QueryField::EnvelopeFromAddress,
            facts
                .envelope_from
                .iter()
                .map(|mailbox| mailbox.address.as_str())
                .collect(),
        ),
        (
            QueryField::EnvelopeFromDomain,
            facts
                .envelope_from
                .iter()
                .map(|mailbox| mailbox.domain.as_str())
                .collect(),
        ),
        (
            QueryField::EnvelopeToAddress,
            facts
                .envelope_to
                .iter()
                .map(|mailbox| mailbox.address.as_str())
                .collect(),
        ),
        (
            QueryField::EnvelopeToDomain,
            facts
                .envelope_to
                .iter()
                .map(|mailbox| mailbox.domain.as_str())
                .collect(),
        ),
        (
            QueryField::FromAddress,
            facts
                .from
                .iter()
                .map(|mailbox| mailbox.address.as_str())
                .collect(),
        ),
        (
            QueryField::FromDomain,
            facts
                .from
                .iter()
                .map(|mailbox| mailbox.domain.as_str())
                .collect(),
        ),
        (
            QueryField::ToAddress,
            facts
                .to
                .iter()
                .map(|mailbox| mailbox.address.as_str())
                .collect(),
        ),
        (
            QueryField::ToDomain,
            facts
                .to
                .iter()
                .map(|mailbox| mailbox.domain.as_str())
                .collect(),
        ),
        (
            QueryField::CarbonCopyAddress,
            facts
                .carbon_copy
                .iter()
                .map(|mailbox| mailbox.address.as_str())
                .collect(),
        ),
        (
            QueryField::CarbonCopyDomain,
            facts
                .carbon_copy
                .iter()
                .map(|mailbox| mailbox.domain.as_str())
                .collect(),
        ),
        (QueryField::Subject, vec![facts.subject.as_str()]),
        (QueryField::Text, vec![facts.text.as_str()]),
        (QueryField::MarkupBody, vec![facts.markup_body.as_str()]),
    ];
    if let Some(identifier) = &facts.message_identifier {
        values.push((QueryField::MessageIdentifier, vec![identifier]));
    }
    for (name, strings) in &facts.headers {
        values.push((
            QueryField::Header(name.clone()),
            strings.iter().map(String::as_str).collect(),
        ));
    }
    values
}

/// Return every literal-search value, including header names, without copying message strings.
fn indexed_content_values(facts: &MessageFacts) -> Vec<&str> {
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
    values.extend(facts.message_identifier.iter().map(String::as_str));
    for (name, header_values) in &facts.headers {
        values.push(name);
        values.extend(header_values.iter().map(String::as_str));
    }
    values
}
