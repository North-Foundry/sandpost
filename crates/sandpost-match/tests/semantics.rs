mod common;

use common::{facts, scope};
use sandpost_core::ScopeTree;
use sandpost_match::Matcher;
use sandpost_query::compile;
use std::collections::{BTreeSet, HashMap};

/// Compare indexed inherited matching against exhaustive direct query evaluation.
#[test]
fn indexed_engine_agrees_with_exhaustive_semantic_oracle() {
    let sources = [
        "",
        "false",
        "from.domain == 'a.dev'",
        "from.domain == 'b.dev'",
        "from.domain == 'a.dev' or to.domain == 'c.dev'",
        "from.domain == 'a.dev' and subject contains 'hello'",
        "not (from.domain == 'a.dev')",
        "to.domain != 'c.dev'",
        "header['X-App'] == 'one'",
        "subject contains 'hello'",
        "(from.domain == 'a.dev' or subject == 'other') and to.domain == 'c.dev'",
    ];
    let roots: Vec<_> = sources.iter().map(|source| scope(None, source)).collect();
    let mut scopes = roots.clone();
    for parent in &roots {
        scopes.extend(
            sources
                .iter()
                .map(|source| scope(Some(parent.identifier), source)),
        );
    }
    let tree = ScopeTree::new(scopes, None).unwrap();
    let matcher = Matcher::new(&tree).unwrap();
    let queries: HashMap<_, _> = tree
        .scopes()
        .map(|scope| (scope.identifier, compile(&scope.filter).unwrap()))
        .collect();
    for from in ["a.dev", "b.dev", "other.dev"] {
        for to in [vec![], vec!["c.dev"], vec!["c.dev", "other.dev"]] {
            for subject in ["hello", "other"] {
                let mut message = facts(from);
                message.subject = subject.into();
                message.to = to
                    .iter()
                    .map(|domain| sandpost_core::Mailbox {
                        address: format!("dev@{domain}"),
                        domain: (*domain).into(),
                    })
                    .collect();
                message
                    .headers
                    .insert("x-app".into(), vec!["one".into(), "two".into()]);
                let expected: BTreeSet<_> = tree
                    .scopes()
                    .filter(|scope| {
                        queries[&scope.identifier].evaluate(&message)
                            && tree
                                .ancestors(scope.identifier)
                                .unwrap()
                                .iter()
                                .all(|parent| queries[parent].evaluate(&message))
                    })
                    .map(|scope| scope.identifier)
                    .collect();
                let actual: BTreeSet<_> = matcher
                    .match_message(&message)
                    .scopes
                    .into_iter()
                    .map(|(identifier, _)| identifier)
                    .collect();
                assert_eq!(
                    actual, expected,
                    "from={from}, to={to:?}, subject={subject}"
                );
            }
        }
    }
}
