# `sandpost-match`

`sandpost-match` compiles a validated `sandpost_core::ScopeTree` into an
immutable in-memory matcher snapshot. It compiles each scope's local filter with
`sandpost_query`, applies inherited restrictions, selects possible matches
from exact message facts, and evaluates shared predicates. It returns matching
scope identifiers with their policy versions; it does not persist results,
authenticate users, publish policy changes, or run historical reclassification
jobs. The input tree must be a valid `ScopeTree` from `sandpost-core`.

```rust
use sandpost_core::{Mailbox, MessageFacts, Scope, ScopeIdentifier, ScopeTree};
use sandpost_match::Matcher;

let scope_identifier = ScopeIdentifier::new();
let scope = Scope {
    identifier: scope_identifier,
    parent: None,
    name: "operations".into(),
    description: None,
    filter: r#"from.domain == "example.test" and subject contains "build""#.into(),
    position: 0,
    policy_version: 1,
};
let tree = ScopeTree::new([scope], None)?;
let matcher = Matcher::new(&tree)?;
let facts = MessageFacts {
    from: vec![Mailbox {
        address: "builds@example.test".into(),
        domain: "example.test".into(),
    }],
    subject: "build report".into(),
    ..MessageFacts::default()
};
let result = matcher.match_message(&facts);
assert_eq!(result.scopes, vec![(scope_identifier, 1)]);
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Matching behavior

Each scope's effective expression is its parent's effective expression AND its
own local filter. This ensures descendants cannot broaden inherited visibility.
Independent roots remain independent. Matching consumes normalized
`MessageFacts`; mailbox facts and header names should already follow core's
normalization rules.

Candidate selection uses a safe positive anchor cover. Exact equality on
envelope sender/recipient address or domain, parsed From/To/Cc address or
domain, a named header, or RFC `message_id` can supply an anchor. For an AND,
any available required cover is sufficient, and the smallest available cover
is chosen. For an OR, every branch needs a cover and their keys are combined.
Inherited covers can anchor a child whose local filter has no anchor. False
expressions have an empty cover and select no candidates. If no safe cover
exists, the scope enters the explicit fallback bucket. That bucket may grow to
O(number of scopes), for example when policies use only negation or body
comparisons; matching correctness does not depend on inventing an unsafe
anchor.

The matcher unions indexed scopes with fallback scopes, then evaluates only
those candidates. A sparse per-message memo shares each visited predicate and
expression result across plans. Boolean children are ordered using query cost
hints, and evaluation short-circuits. Scope traversal and expression evaluation
use iterative stacks; construction of the bounded query expression tree is
recursive.

`MatchStatistics` reports operation counts for one message: `candidates` is
the number of unique scopes selected for evaluation,
`predicate_evaluations` counts predicate nodes actually evaluated after memo
reuse, and `expression_evaluations` counts expression nodes actually
evaluated after memo reuse. These counts describe work shape, not elapsed time
or throughput. `Matcher::shared_node_count` reports distinct interned
expression nodes in the snapshot. `Matcher::fallback_scope_count` reports
scopes without a safe positive anchor.

The returned `MatchResult::scopes` contains `(ScopeIdentifier, policy_version)`
pairs in deterministic identifier order. Consumers can use the version to
associate a result with the policy snapshot that produced it. A matcher is an
immutable snapshot: after editing the tree, build a new matcher to use the
updated policies. Snapshot construction does not itself publish a policy or
refresh persisted visibility.

`MatchError::Tree` reports a core tree traversal or lookup error.
`MatchError::Query` identifies the scope whose filter failed query compilation
and retains the underlying `QueryError`.

`MatchDelta::between(old, new)` computes sorted message-sequence set
differences: `added` is `new - old`, and `removed` is `old - new`. It is an
in-memory value operation; storage transactions and job scheduling belong to
other layers.

## Module map

- `src/lib.rs`: module declarations and unchanged root exports: `Matcher`,
  `MatchError`, `MatchStatistics`, `MatchResult`, and `MatchDelta`.
- `src/matcher.rs`: immutable snapshot compilation, candidate selection,
  matching results, statistics, and matcher errors.
- `src/matcher/evaluation.rs`: shared expression interning and iterative,
  memoized evaluation.
- `src/candidates.rs`: exact positive anchor cover construction and candidate
  index helpers.
- `src/delta.rs`: sorted message-sequence set differences.
- `tests/common/`: shared scope and normalized-message fixtures.
- `tests/candidates.rs`: candidate completeness, anchor selection, and shared predicates.
- `tests/inheritance.rs`: inherited AND semantics, branch independence, and
  short-circuit behavior, including deep hierarchies.
- `tests/semantics.rs`: matcher agreement with exhaustive direct query evaluation.
- `tests/delta.rs`: added and removed sequence sets.

The 1,001 exact-domain scope case checks that two scopes become candidates and
that their identical predicate is evaluated once. It verifies candidate and
memoization behavior; it is not a throughput benchmark.

## Verification

```sh
cargo test --locked -p sandpost-match
cargo clippy --locked -p sandpost-match --all-targets -- -D warnings
```
