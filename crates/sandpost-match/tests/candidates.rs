mod common;

use common::{facts, scope};
use sandpost_core::ScopeTree;
use sandpost_match::Matcher;

/// Verify exact indexing limits candidates and identical predicates execute once.
#[test]
fn indexed_candidates_and_predicate_sharing() {
    let mut scopes: Vec<_> = (0..1000)
        .map(|scope_number| scope(None, &format!("from.domain == \"{scope_number}.dev\"")))
        .collect();
    scopes.push(scope(None, "from.domain == \"7.dev\""));
    let matcher = Matcher::new(&ScopeTree::new(scopes, None).unwrap()).unwrap();
    let result = matcher.match_message(&facts("7.dev"));
    assert_eq!(result.scopes.len(), 2);
    assert_eq!(result.statistics.candidates, 2);
    assert_eq!(result.statistics.predicate_evaluations, 1);
    assert_eq!(matcher.fallback_scope_count(), 0);
}

/// Verify disjunction and negative filters never lose matching candidates.
#[test]
fn or_and_negative_filters_have_complete_candidate_coverage() {
    let mixed_filter_scope = scope(
        None,
        "from.domain == \"a.dev\" or subject contains \"hello\"",
    );
    let negative_filter_scope = scope(None, "not from.domain == \"b.dev\"");
    let domain_filter_scope = scope(None, "from.domain == \"a.dev\" or from.domain == \"c.dev\"");
    let matcher = Matcher::new(
        &ScopeTree::new(
            [
                mixed_filter_scope,
                negative_filter_scope,
                domain_filter_scope,
            ],
            None,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(matcher.match_message(&facts("x.dev")).scopes.len(), 2);
    assert_eq!(matcher.match_message(&facts("c.dev")).scopes.len(), 3);
}
