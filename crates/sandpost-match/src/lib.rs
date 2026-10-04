//! Immutable indexed policy snapshot with shared predicate/expression nodes.
//! Storage publishes matching relationships transactionally after evaluation.
use sandpost_core::{MessageFacts, MessageSequence, ScopeIdentifier, ScopeTree, TreeError};
use sandpost_query::{Expression, Field, Operator, Predicate, QueryError, Value, compile};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Debug, thiserror::Error)]
pub enum MatchError {
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error("invalid filter for scope {scope}: {source}")]
    Query {
        scope: ScopeIdentifier,
        source: QueryError,
    },
}

type NodeIdentifier = usize;
type CandidateAnchor = (Field, Value);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum EvaluationNode {
    Constant(bool),
    Predicate(Predicate),
    Not(NodeIdentifier),
    And(Vec<NodeIdentifier>),
    Or(Vec<NodeIdentifier>),
}

#[derive(Debug, Clone, Copy)]
struct ScopePlan {
    node: NodeIdentifier,
    version: u64,
}

#[derive(Debug, Default)]
pub struct MatchStatistics {
    pub candidates: usize,
    pub predicate_evaluations: usize,
    pub expression_evaluations: usize,
}

#[derive(Debug, Default)]
pub struct MatchResult {
    pub scopes: Vec<(ScopeIdentifier, u64)>,
    pub statistics: MatchStatistics,
}

/// Build once per policy snapshot, reuse on each ingest. Exact positive predicates
/// drive selection. Unanchored policies form an explicit fallback bucket.
/// ponytail: body-only/negative policies can make fallback O(scopes); add
/// selective token/set indexes when representative policies justify them.
pub struct Matcher {
    nodes: Vec<EvaluationNode>,
    interned: HashMap<EvaluationNode, NodeIdentifier>,
    costs: Vec<u8>,
    plans: HashMap<ScopeIdentifier, ScopePlan>,
    anchors: HashMap<CandidateAnchor, Vec<ScopeIdentifier>>,
    fields: HashSet<Field>,
    fallback: Vec<ScopeIdentifier>,
}

impl Matcher {
    /// Compile inherited scope filters into an immutable shared evaluation and candidate plan.
    pub fn new(tree: &ScopeTree) -> Result<Self, MatchError> {
        let mut engine = Self {
            nodes: Vec::new(),
            interned: HashMap::new(),
            costs: Vec::new(),
            plans: HashMap::new(),
            anchors: HashMap::new(),
            fields: HashSet::new(),
            fallback: Vec::new(),
        };
        let mut covers: HashMap<ScopeIdentifier, Option<BTreeSet<CandidateAnchor>>> =
            HashMap::new();
        // Preorder ensures parents exist before children, without recursive traversal.
        for root in tree.roots() {
            for identifier in tree.subtree(*root)? {
                let scope = tree.get(identifier).ok_or(TreeError::Unknown(identifier))?;
                let query = compile(&scope.filter).map_err(|source| MatchError::Query {
                    scope: identifier,
                    source,
                })?;
                let local = engine.intern_expression(query.expression());
                let mut cover = anchor_cover(query.expression());
                let node = if let Some(parent) = scope.parent {
                    cover = intersect_cover(covers.get(&parent).cloned().flatten(), cover);
                    let parent_node = engine.plans[&parent].node;
                    engine.intern(EvaluationNode::And(vec![parent_node, local]))
                } else {
                    local
                };
                engine.plans.insert(
                    identifier,
                    ScopePlan {
                        node,
                        version: scope.policy_version,
                    },
                );
                if let Some(keys) = &cover {
                    for key in keys {
                        engine.fields.insert(key.0.clone());
                        engine
                            .anchors
                            .entry(key.clone())
                            .or_default()
                            .push(identifier);
                    }
                } else {
                    engine.fallback.push(identifier);
                }
                covers.insert(identifier, cover);
            }
        }
        Ok(engine)
    }

    /// Intern the canonical expression recursively, sharing equivalent child nodes.
    fn intern_expression(&mut self, expression: &Expression) -> NodeIdentifier {
        let node = match expression {
            Expression::True => EvaluationNode::Constant(true),
            Expression::False => EvaluationNode::Constant(false),
            Expression::Predicate(predicate) => EvaluationNode::Predicate(predicate.clone()),
            Expression::Not(child) => EvaluationNode::Not(self.intern_expression(child)),
            Expression::And(children) => EvaluationNode::And(
                children
                    .iter()
                    .map(|child| self.intern_expression(child))
                    .collect(),
            ),
            Expression::Or(children) => EvaluationNode::Or(
                children
                    .iter()
                    .map(|child| self.intern_expression(child))
                    .collect(),
            ),
        };
        self.intern(node)
    }

    /// Order boolean children by cost and reuse an equivalent node when one exists.
    fn intern(&mut self, mut node: EvaluationNode) -> NodeIdentifier {
        if let EvaluationNode::And(children) | EvaluationNode::Or(children) = &mut node {
            children.sort_unstable_by_key(|identifier| (self.costs[*identifier], *identifier));
            children.dedup();
            if children.len() == 1 {
                return children[0];
            }
        }
        if let Some(identifier) = self.interned.get(&node) {
            return *identifier;
        }
        let cost = match &node {
            EvaluationNode::Constant(_) => 0,
            EvaluationNode::Predicate(predicate) => predicate.cost(),
            EvaluationNode::Not(identifier) => self.costs[*identifier],
            EvaluationNode::And(identifiers) | EvaluationNode::Or(identifiers) => identifiers
                .iter()
                .map(|identifier| self.costs[*identifier])
                .max()
                .unwrap_or(0),
        };
        let identifier = self.nodes.len();
        self.interned.insert(node.clone(), identifier);
        self.nodes.push(node);
        self.costs.push(cost);
        identifier
    }

    /// Select candidates, evaluate shared nodes once, and return matching policy versions.
    pub fn match_message(&self, facts: &MessageFacts) -> MatchResult {
        let mut candidates: BTreeSet<ScopeIdentifier> = self.fallback.iter().copied().collect();
        for field in &self.fields {
            for value in field.values(facts) {
                if let Some(identifiers) = self.anchors.get(&(field.clone(), value)) {
                    candidates.extend(identifiers);
                }
            }
        }
        let mut result = MatchResult::default();
        result.statistics.candidates = candidates.len();
        // Sparse per-message cache: allocation proportional to visited nodes only.
        let mut evaluation_cache = HashMap::new();
        for identifier in candidates {
            let plan = self.plans[&identifier];
            if self.evaluate(
                plan.node,
                facts,
                &mut evaluation_cache,
                &mut result.statistics,
            ) {
                result.scopes.push((identifier, plan.version));
            }
        }
        result
    }

    /// Explicit evaluation stack supports arbitrarily deep scope hierarchies.
    fn evaluate(
        &self,
        root: NodeIdentifier,
        facts: &MessageFacts,
        evaluation_cache: &mut HashMap<NodeIdentifier, bool>,
        statistics: &mut MatchStatistics,
    ) -> bool {
        let mut stack = vec![(root, 0usize)];
        while let Some((identifier, next_child)) = stack.last().copied() {
            if evaluation_cache.contains_key(&identifier) {
                stack.pop();
                continue;
            }
            let value = match &self.nodes[identifier] {
                EvaluationNode::Constant(value) => Some(*value),
                EvaluationNode::Predicate(predicate) => {
                    statistics.predicate_evaluations += 1;
                    Some(predicate.evaluate(facts))
                }
                EvaluationNode::Not(child) => match evaluation_cache.get(child) {
                    Some(value) => Some(!value),
                    None => {
                        stack.push((*child, 0));
                        None
                    }
                },
                EvaluationNode::And(children) | EvaluationNode::Or(children) => {
                    let is_and = matches!(self.nodes[identifier], EvaluationNode::And(_));
                    if next_child == children.len() {
                        Some(is_and)
                    } else {
                        let child = children[next_child];
                        match evaluation_cache.get(&child) {
                            Some(value) if *value != is_and => Some(*value),
                            Some(_) => {
                                if let Some(frame) = stack.last_mut() {
                                    frame.1 += 1;
                                }
                                None
                            }
                            None => {
                                stack.push((child, 0));
                                None
                            }
                        }
                    }
                }
            };
            if let Some(value) = value {
                evaluation_cache.insert(identifier, value);
                statistics.expression_evaluations += 1;
                stack.pop();
            }
        }
        evaluation_cache.get(&root).copied().unwrap_or(false)
    }

    /// Return the number of distinct interned nodes in this policy snapshot.
    pub fn shared_node_count(&self) -> usize {
        self.nodes.len()
    }
    /// Return the number of scopes without a safe positive index anchor.
    pub fn fallback_scope_count(&self) -> usize {
        self.fallback.len()
    }
}

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
fn anchor_cover(expression: &Expression) -> Option<BTreeSet<CandidateAnchor>> {
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
fn intersect_cover(
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

/// Set representation can be replaced by compressed bitmaps without changing
/// the semantic delta contract. No mailbox or inbox owns copies of messages.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MatchDelta {
    pub added: BTreeSet<MessageSequence>,
    pub removed: BTreeSet<MessageSequence>,
}
impl MatchDelta {
    /// Compute added and removed message sequences relative to the previous match set.
    pub fn between(old: &BTreeSet<MessageSequence>, new: &BTreeSet<MessageSequence>) -> Self {
        Self {
            added: new.difference(old).copied().collect(),
            removed: old.difference(new).copied().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandpost_core::{Mailbox, Scope};
    /// Build a scope fixture from its parent and local filter.
    fn scope(parent: Option<ScopeIdentifier>, filter: &str) -> Scope {
        Scope {
            identifier: ScopeIdentifier::new(),
            parent,
            name: "test".into(),
            description: None,
            filter: filter.into(),
            position: 0,
            policy_version: 1,
        }
    }
    /// Build normalized sender facts for the given domain.
    fn facts(domain: &str) -> MessageFacts {
        MessageFacts {
            from: vec![Mailbox {
                address: format!("dev@{domain}"),
                domain: domain.into(),
            }],
            subject: "hello".into(),
            ..MessageFacts::default()
        }
    }
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
    /// Verify disjunction and negative filters never lose matching candidates.
    #[test]
    fn or_and_negative_filters_have_complete_candidate_coverage() {
        let mixed_filter_scope = scope(
            None,
            "from.domain == \"a.dev\" or subject contains \"hello\"",
        );
        let negative_filter_scope = scope(None, "not from.domain == \"b.dev\"");
        let domain_filter_scope =
            scope(None, "from.domain == \"a.dev\" or from.domain == \"c.dev\"");
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
    /// Verify message-set differences preserve only additions and removals.
    #[test]
    fn set_deltas() {
        let old = BTreeSet::from([MessageSequence(1), MessageSequence(2)]);
        let new = BTreeSet::from([MessageSequence(2), MessageSequence(3)]);
        assert_eq!(
            MatchDelta::between(&old, &new),
            MatchDelta {
                added: BTreeSet::from([MessageSequence(3)]),
                removed: BTreeSet::from([MessageSequence(1)])
            }
        );
    }
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
}
