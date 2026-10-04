mod common;

use common::{facts, scope};
use sandpost_core::ScopeTree;
use sandpost_match::Matcher;

/// Verify inherited restrictions and unrelated sibling scopes remain independent.
#[test]
fn inherited_filters_cannot_expand_and_subtrees_stay_separate() {
    let parent = scope(None, "from.domain == \"a.dev\"");
    let child = scope(Some(parent.identifier), "subject contains \"hello\"");
    let sibling = scope(None, "from.domain == \"b.dev\"");
    let tree = ScopeTree::new([parent.clone(), child.clone(), sibling.clone()], None).unwrap();
    let matcher = Matcher::new(&tree).unwrap();
    let result = matcher.match_message(&facts("b.dev"));
    assert_eq!(result.scopes, vec![(sibling.identifier, 1)]);
    assert_eq!(result.statistics.candidates, 1);
    assert_eq!(
        tree.subtree(child.identifier).unwrap(),
        vec![child.identifier]
    );
}

/// Verify a rejected parent skips expensive descendant predicates.
#[test]
fn parent_failure_short_circuits_expensive_descendants() {
    let parent = scope(None, "subject == \"never\"");
    let child = scope(Some(parent.identifier), "text contains \"expensive\"");
    let matcher = Matcher::new(&ScopeTree::new([parent, child], None).unwrap()).unwrap();
    let result = matcher.match_message(&facts("a.dev"));
    assert!(result.scopes.is_empty());
    assert_eq!(result.statistics.predicate_evaluations, 1);
}

/// Verify deep inheritance uses an explicit stack and shares repeated predicates.
#[test]
fn deep_inheritance_evaluates_without_recursion_or_duplicate_predicates() {
    let root = scope(None, "from.domain == 'a.dev'");
    let mut parent = root.identifier;
    let mut scopes = vec![root];
    for _ in 0..1500 {
        let child = scope(Some(parent), "subject contains 'hello'");
        parent = child.identifier;
        scopes.push(child);
    }
    let matcher = Matcher::new(&ScopeTree::new(scopes, None).unwrap()).unwrap();
    let result = matcher.match_message(&facts("a.dev"));
    assert_eq!(result.scopes.len(), 1501);
    assert_eq!(result.statistics.predicate_evaluations, 2);
    assert!(matcher.match_message(&facts("other.dev")).scopes.is_empty());
}
