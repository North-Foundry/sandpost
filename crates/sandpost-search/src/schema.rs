//! Fixed private index schema; stored facts and indexed columns have distinct responsibilities.

use tantivy::schema::{FAST, Field, INDEXED, STORED, STRING, Schema};

/// Tantivy field handles kept private so Tantivy types stay out of the API.
#[derive(Clone, Copy)]
pub(super) struct SearchFields {
    pub(super) identifier: Field,
    pub(super) sequence: Field,
    pub(super) revision: Field,
    pub(super) facts: Field,
    pub(super) exact_values: Field,
    pub(super) trigrams: Field,
    pub(super) received_at: Field,
    pub(super) size: Field,
    pub(super) attachment_count: Field,
}

/// Build the fixed schema used by new index directories.
pub(super) fn schema() -> (Schema, SearchFields) {
    let mut builder = Schema::builder();
    let identifier = builder.add_text_field("identifier", STRING | FAST);
    let sequence = builder.add_u64_field("sequence", INDEXED | FAST);
    let revision = builder.add_u64_field("revision", FAST);
    let facts = builder.add_text_field("facts_json", STORED);
    let exact_values = builder.add_text_field("exact_values", STRING);
    let trigrams = builder.add_text_field("trigrams", STRING);
    let received_at = builder.add_i64_field("received_at", INDEXED | FAST);
    let size = builder.add_u64_field("size", INDEXED | FAST);
    let attachment_count = builder.add_u64_field("attachment_count", INDEXED | FAST);
    (
        builder.build(),
        SearchFields {
            identifier,
            sequence,
            revision,
            facts,
            exact_values,
            trigrams,
            received_at,
            size,
            attachment_count,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keep stored payloads minimal and result metadata available without reading message bodies.
    #[test]
    fn only_canonical_facts_are_stored() {
        let (schema, _) = schema();
        let stored_fields: Vec<_> = schema
            .fields()
            .filter(|(_, entry)| entry.is_stored())
            .map(|(_, entry)| entry.name())
            .collect();
        assert_eq!(stored_fields, ["facts_json"]);
        assert_eq!(schema.num_fields(), 9);
        for name in [
            "identifier",
            "sequence",
            "revision",
            "received_at",
            "size",
            "attachment_count",
        ] {
            assert!(
                schema
                    .get_field_entry(schema.get_field(name).unwrap())
                    .is_fast(),
                "{name}"
            );
        }
        for name in ["attachments_json", "document_kind"] {
            assert!(schema.get_field(name).is_err(), "{name}");
        }
    }

    /// Reject a legacy generation explicitly so the application can rebuild it from storage.
    #[test]
    fn incompatible_generation_requires_rebuild() {
        let directory = std::env::temp_dir().join(format!(
            "sandpost-search-legacy-{}",
            sandpost_core::MessageIdentifier::new()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let mut legacy = Schema::builder();
        legacy.add_text_field("attachments_json", STORED);
        drop(tantivy::Index::create_in_dir(&directory, legacy.build()).unwrap());
        assert!(matches!(
            crate::SearchIndex::open(&directory),
            Err(crate::SearchError::IncompatibleSchema)
        ));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
