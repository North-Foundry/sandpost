//! Snapshot search, exact-expression fast paths, and verified keyset pagination.

use crate::{
    MessageQuery, SearchError,
    query::{MAXIMUM_SEARCH_BATCH_SIZE, candidate_query, requires_verification},
    schema::SearchFields,
};
use sandpost_core::{MessageFacts, MessageIdentifier};
use sandpost_query::Expression;
use tantivy::{
    IndexReader, TantivyDocument, TantivyError, Term,
    collector::TopDocs,
    columnar::{Column, StrColumn},
    query::{BooleanQuery, Occur, Query, RangeQuery, TermQuery},
    schema::{IndexRecordOption, Value},
};

/// Read result identity and revision without fetching stored message bodies.
struct HitColumns {
    identifier: StrColumn,
    revision: Column<u64>,
}

impl HitColumns {
    /// Open each segment's result columns once for the lifetime of the search snapshot.
    fn open(segment: &tantivy::SegmentReader) -> Result<Self, TantivyError> {
        Ok(Self {
            identifier: segment
                .fast_fields()
                .str("identifier")?
                .ok_or_else(|| missing_field("identifier"))?,
            revision: segment.fast_fields().u64("revision")?,
        })
    }

    /// Decode one public identifier and its immutable indexed revision.
    fn hit(
        &self,
        document: u32,
        identifier_buffer: &mut String,
    ) -> Result<(MessageIdentifier, u64), SearchError> {
        let ordinal = self
            .identifier
            .ords()
            .first(document)
            .ok_or_else(|| missing_field("identifier"))?;
        if !self.identifier.ord_to_str(ordinal, identifier_buffer)? {
            return Err(missing_field("identifier").into());
        }
        let identifier = identifier_buffer.parse().map_err(|error| {
            TantivyError::InvalidArgument(format!("invalid identifier: {error}"))
        })?;
        let revision = self
            .revision
            .first(document)
            .ok_or_else(|| missing_field("revision"))?;
        Ok((identifier, revision))
    }
}

/// Execute one request against a stable reader snapshot, newest first.
pub(super) fn search_hits(
    reader: &IndexReader,
    fields: SearchFields,
    request: &MessageQuery,
) -> Result<Vec<(MessageIdentifier, u64)>, SearchError> {
    let result_limit = request.limit.min(MAXIMUM_SEARCH_BATCH_SIZE);
    if result_limit == 0
        || matches!(request.authorization, Some(Expression::False))
        || matches!(request.filter, Expression::False)
        || request.before.is_some_and(|sequence| sequence.0 == 0)
    {
        return Ok(Vec::new());
    }
    let verify_facts = requires_verification(&request.filter)
        || request
            .authorization
            .as_ref()
            .is_some_and(requires_verification);
    let mut required = vec![(Occur::Must, candidate_query(&request.filter, fields))];
    if let Some(authorization) = &request.authorization {
        required.push((Occur::Must, candidate_query(authorization, fields)));
    }
    if let Some(identifier) = request.message_identifier {
        required.push((
            Occur::Must,
            Box::new(TermQuery::new(
                Term::from_field_text(fields.identifier, &identifier.to_string()),
                IndexRecordOption::Basic,
            )),
        ));
    }
    let query = BooleanQuery::new(required);
    let searcher = reader.searcher();
    let columns = searcher
        .segment_readers()
        .iter()
        .map(HitColumns::open)
        .collect::<Result<Vec<_>, _>>()?;
    let mut results = Vec::with_capacity(result_limit);
    let mut upper_sequence = request.before.map(|sequence| sequence.0);
    let page_size = if verify_facts {
        MAXIMUM_SEARCH_BATCH_SIZE
    } else {
        result_limit
    };
    let mut identifier_buffer = String::with_capacity(36);
    while results.len() < result_limit {
        let mut page_clauses = vec![(Occur::Must, Box::new(query.clone()) as Box<dyn Query>)];
        if let Some(upper_sequence) = upper_sequence {
            let Some(inclusive_upper) = upper_sequence.checked_sub(1) else {
                break;
            };
            page_clauses.push((
                Occur::Must,
                Box::new(RangeQuery::new(
                    std::ops::Bound::Unbounded,
                    std::ops::Bound::Included(Term::from_field_u64(
                        fields.sequence,
                        inclusive_upper,
                    )),
                )),
            ));
        }
        let hits = searcher.search(
            &BooleanQuery::new(page_clauses),
            &TopDocs::with_limit(page_size)
                .order_by_fast_field::<u64>("sequence", tantivy::Order::Desc),
        )?;
        if hits.is_empty() {
            break;
        }
        for (sequence, address) in &hits {
            upper_sequence = Some(sequence.ok_or_else(|| missing_field("sequence"))?);
            if verify_facts {
                let document: TantivyDocument = searcher.doc(*address)?;
                if !matches_request(&document, fields, request)? {
                    continue;
                }
            }
            results.push(
                columns[address.segment_ord as usize]
                    .hit(address.doc_id, &mut identifier_buffer)?,
            );
            if results.len() == result_limit {
                break;
            }
        }
        if hits.len() < page_size {
            break;
        }
    }
    Ok(results)
}

/// Evaluate authorization and filter against the canonical stored facts for approximate plans.
fn matches_request(
    document: &TantivyDocument,
    fields: SearchFields,
    request: &MessageQuery,
) -> Result<bool, SearchError> {
    let facts: MessageFacts = serde_json::from_str(
        document
            .get_first(fields.facts)
            .and_then(|value| value.as_str())
            .ok_or_else(|| missing_field("facts"))?,
    )?;
    Ok(request
        .authorization
        .as_ref()
        .is_none_or(|authorization| authorization.evaluate(&facts))
        && request.filter.evaluate(&facts))
}

/// Report missing required index data as an explicit search failure.
fn missing_field(field: &str) -> TantivyError {
    TantivyError::InvalidArgument(format!("missing {field}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandpost_query::{Field, Operator, Predicate, Value as QueryValue};

    /// Prove exact plans avoid stored payload reads while approximate and invalid plans require them.
    #[test]
    fn exact_filters_do_not_decode_stored_bodies() {
        let (schema, fields) = crate::schema::schema();
        let index = tantivy::Index::create_in_ram(schema);
        let mut writer = index.writer_with_num_threads(1, 15_000_000).unwrap();
        let identifier = MessageIdentifier::new();
        let mut document = TantivyDocument::default();
        document.add_text(fields.identifier, identifier.to_string());
        document.add_u64(fields.sequence, 1);
        document.add_u64(fields.revision, 7);
        document.add_u64(fields.size, 1024);
        document.add_i64(fields.received_at, 0);
        document.add_u64(fields.attachment_count, 0);
        document.add_text(
            fields.facts,
            "invalid JSON proves the payload is not decoded",
        );
        writer.add_document(document).unwrap();
        writer.commit().unwrap();
        let reader: IndexReader = index.reader().unwrap();
        let mut request = MessageQuery {
            authorization: None,
            filter: Expression::True,
            message_identifier: None,
            before: None,
            limit: 1,
        };
        assert_eq!(
            search_hits(&reader, fields, &request).unwrap(),
            [(identifier, 7)]
        );
        let numeric = Expression::Predicate(Predicate {
            field: Field::Size,
            operator: Operator::Equal,
            value: QueryValue::Number(1024),
        });
        request.filter = numeric.clone();
        request.authorization = Some(numeric);
        assert_eq!(
            search_hits(&reader, fields, &request).unwrap(),
            [(identifier, 7)]
        );
        request.filter = Expression::Not(Box::new(Expression::False));
        assert!(matches!(
            search_hits(&reader, fields, &request),
            Err(SearchError::Serialization(_))
        ));
        request.filter = Expression::Predicate(Predicate {
            field: Field::Size,
            operator: Operator::Contains,
            value: QueryValue::Number(1024),
        });
        assert!(matches!(
            search_hits(&reader, fields, &request),
            Err(SearchError::Serialization(_))
        ));
    }
}
