//! Immutable indexed policy snapshot with shared predicate/expression nodes.
//! Storage publishes matching relationships transactionally after evaluation.
use sandpost_core::{MessageFacts, MessageSeq, ScopeId, ScopeTree, TreeError};
use sandpost_query::{Expr, Field, Operator, Predicate, QueryError, Value, compile};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Debug, thiserror::Error)]
pub enum MatchError {
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error("invalid filter for scope {scope}: {source}")]
    Query { scope: ScopeId, source: QueryError },
}

type NodeId = usize;
type Anchor = (Field, Value);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Node {
    Constant(bool),
    Predicate(Predicate),
    Not(NodeId),
    And(Vec<NodeId>),
    Or(Vec<NodeId>),
}

#[derive(Debug, Clone, Copy)]
struct Plan {
    node: NodeId,
    version: u64,
}

#[derive(Debug, Default)]
pub struct MatchStats {
    pub candidates: usize,
    pub predicate_evaluations: usize,
    pub expression_evaluations: usize,
}

#[derive(Debug, Default)]
pub struct MatchResult {
    pub scopes: Vec<(ScopeId, u64)>,
    pub stats: MatchStats,
}

/// Build once per policy snapshot, reuse on each ingest. Exact positive predicates
/// drive selection. Unanchored policies form an explicit fallback bucket.
/// ponytail: body-only/negative policies can make fallback O(scopes); add
/// selective token/set indexes when representative policies justify them.
pub struct Matcher {
    nodes: Vec<Node>,
    interned: HashMap<Node, NodeId>,
    costs: Vec<u8>,
    plans: HashMap<ScopeId, Plan>,
    anchors: HashMap<Anchor, Vec<ScopeId>>,
    fields: HashSet<Field>,
    fallback: Vec<ScopeId>,
}

impl Matcher {
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
        let mut covers: HashMap<ScopeId, Option<BTreeSet<Anchor>>> = HashMap::new();
        // Preorder ensures parents exist before children, without recursive traversal.
        for root in tree.roots() {
            for id in tree.subtree(*root)? {
                let scope = tree.get(id).ok_or(TreeError::Unknown(id))?;
                let query = compile(&scope.filter)
                    .map_err(|source| MatchError::Query { scope: id, source })?;
                let local = engine.intern_expr(&query.expression);
                let mut cover = anchor_cover(&query.expression);
                let node = if let Some(parent) = scope.parent {
                    cover = intersect_cover(covers.get(&parent).cloned().flatten(), cover);
                    let parent_node = engine.plans[&parent].node;
                    engine.intern(Node::And(vec![parent_node, local]))
                } else {
                    local
                };
                engine.plans.insert(
                    id,
                    Plan {
                        node,
                        version: scope.policy_version,
                    },
                );
                if let Some(keys) = &cover {
                    for key in keys {
                        engine.fields.insert(key.0.clone());
                        engine.anchors.entry(key.clone()).or_default().push(id);
                    }
                } else {
                    engine.fallback.push(id);
                }
                covers.insert(id, cover);
            }
        }
        Ok(engine)
    }

    fn intern_expr(&mut self, expr: &Expr) -> NodeId {
        let node = match expr {
            Expr::True => Node::Constant(true),
            Expr::False => Node::Constant(false),
            Expr::Predicate(p) => Node::Predicate(p.clone()),
            Expr::Not(child) => Node::Not(self.intern_expr(child)),
            Expr::And(children) => Node::And(
                children
                    .iter()
                    .map(|child| self.intern_expr(child))
                    .collect(),
            ),
            Expr::Or(children) => Node::Or(
                children
                    .iter()
                    .map(|child| self.intern_expr(child))
                    .collect(),
            ),
        };
        self.intern(node)
    }

    fn intern(&mut self, mut node: Node) -> NodeId {
        if let Node::And(children) | Node::Or(children) = &mut node {
            children.sort_unstable_by_key(|id| (self.costs[*id], *id));
            children.dedup();
            if children.len() == 1 {
                return children[0];
            }
        }
        if let Some(id) = self.interned.get(&node) {
            return *id;
        }
        let cost = match &node {
            Node::Constant(_) => 0,
            Node::Predicate(p) => p.cost(),
            Node::Not(id) => self.costs[*id],
            Node::And(ids) | Node::Or(ids) => {
                ids.iter().map(|id| self.costs[*id]).max().unwrap_or(0)
            }
        };
        let id = self.nodes.len();
        self.interned.insert(node.clone(), id);
        self.nodes.push(node);
        self.costs.push(cost);
        id
    }

    pub fn match_message(&self, facts: &MessageFacts) -> MatchResult {
        let mut candidates: BTreeSet<ScopeId> = self.fallback.iter().copied().collect();
        for field in &self.fields {
            for value in field.values(facts) {
                if let Some(ids) = self.anchors.get(&(field.clone(), value)) {
                    candidates.extend(ids);
                }
            }
        }
        let mut result = MatchResult::default();
        result.stats.candidates = candidates.len();
        // Sparse per-message cache: allocation proportional to visited nodes only.
        let mut memo = HashMap::new();
        for id in candidates {
            let plan = self.plans[&id];
            if self.evaluate(plan.node, facts, &mut memo, &mut result.stats) {
                result.scopes.push((id, plan.version));
            }
        }
        result
    }

    /// Explicit evaluation stack supports arbitrarily deep scope hierarchies.
    fn evaluate(
        &self,
        root: NodeId,
        facts: &MessageFacts,
        memo: &mut HashMap<NodeId, bool>,
        stats: &mut MatchStats,
    ) -> bool {
        let mut stack = vec![(root, 0usize)];
        while let Some((id, next_child)) = stack.last().copied() {
            if memo.contains_key(&id) {
                stack.pop();
                continue;
            }
            let value = match &self.nodes[id] {
                Node::Constant(value) => Some(*value),
                Node::Predicate(predicate) => {
                    stats.predicate_evaluations += 1;
                    Some(predicate.evaluate(facts))
                }
                Node::Not(child) => match memo.get(child) {
                    Some(value) => Some(!value),
                    None => {
                        stack.push((*child, 0));
                        None
                    }
                },
                Node::And(children) | Node::Or(children) => {
                    let is_and = matches!(self.nodes[id], Node::And(_));
                    if next_child == children.len() {
                        Some(is_and)
                    } else {
                        let child = children[next_child];
                        match memo.get(&child) {
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
                memo.insert(id, value);
                stats.expression_evaluations += 1;
                stack.pop();
            }
        }
        memo.get(&root).copied().unwrap_or(false)
    }

    pub fn shared_node_count(&self) -> usize {
        self.nodes.len()
    }
    pub fn fallback_scope_count(&self) -> usize {
        self.fallback.len()
    }
}

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
            | Field::CcAddress
            | Field::CcDomain
            | Field::Header(_)
            | Field::MessageId
    )
}

/// A cover is a set of exact keys, at least one of which every matching message
/// must contain. None means no safe index anchor. An empty set means impossible.
fn anchor_cover(expr: &Expr) -> Option<BTreeSet<Anchor>> {
    match expr {
        Expr::False => Some(BTreeSet::new()),
        Expr::Predicate(p) if p.op == Operator::Eq && indexable(&p.field) => {
            Some(BTreeSet::from([(p.field.clone(), p.value.clone())]))
        }
        Expr::And(children) => children
            .iter()
            .map(anchor_cover)
            .fold(None, intersect_cover),
        Expr::Or(children) => {
            let mut keys = BTreeSet::new();
            for child in children {
                keys.extend(anchor_cover(child)?);
            }
            Some(keys)
        }
        _ => None,
    }
}

fn intersect_cover(
    a: Option<BTreeSet<Anchor>>,
    b: Option<BTreeSet<Anchor>>,
) -> Option<BTreeSet<Anchor>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if a.len() <= b.len() { a } else { b }),
        (Some(keys), None) | (None, Some(keys)) => Some(keys),
        (None, None) => None,
    }
}

/// Set representation can be replaced by compressed bitmaps without changing
/// the semantic delta contract. No mailbox or inbox owns copies of messages.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MatchDelta {
    pub added: BTreeSet<MessageSeq>,
    pub removed: BTreeSet<MessageSeq>,
}
impl MatchDelta {
    pub fn between(old: &BTreeSet<MessageSeq>, new: &BTreeSet<MessageSeq>) -> Self {
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
    fn scope(parent: Option<ScopeId>, filter: &str) -> Scope {
        Scope {
            id: ScopeId::new(),
            parent,
            name: "test".into(),
            description: None,
            filter: filter.into(),
            position: 0,
            policy_version: 1,
        }
    }
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
    #[test]
    fn indexed_candidates_and_predicate_sharing() {
        let mut scopes: Vec<_> = (0..1000)
            .map(|i| scope(None, &format!("from.domain == \"{i}.dev\"")))
            .collect();
        scopes.push(scope(None, "from.domain == \"7.dev\""));
        let matcher = Matcher::new(&ScopeTree::new(scopes, None).unwrap()).unwrap();
        let result = matcher.match_message(&facts("7.dev"));
        assert_eq!(result.scopes.len(), 2);
        assert_eq!(result.stats.candidates, 2);
        assert_eq!(result.stats.predicate_evaluations, 1);
        assert_eq!(matcher.fallback_scope_count(), 0);
    }
    #[test]
    fn inherited_filters_cannot_expand_and_subtrees_stay_separate() {
        let parent = scope(None, "from.domain == \"a.dev\"");
        let child = scope(Some(parent.id), "subject contains \"hello\"");
        let sibling = scope(None, "from.domain == \"b.dev\"");
        let tree = ScopeTree::new([parent.clone(), child.clone(), sibling.clone()], None).unwrap();
        let matcher = Matcher::new(&tree).unwrap();
        let result = matcher.match_message(&facts("b.dev"));
        assert_eq!(result.scopes, vec![(sibling.id, 1)]);
        assert_eq!(result.stats.candidates, 1);
        assert_eq!(tree.subtree(child.id).unwrap(), vec![child.id]);
    }
    #[test]
    fn or_and_negative_filters_have_complete_candidate_coverage() {
        let a = scope(
            None,
            "from.domain == \"a.dev\" or subject contains \"hello\"",
        );
        let b = scope(None, "not from.domain == \"b.dev\"");
        let c = scope(None, "from.domain == \"a.dev\" or from.domain == \"c.dev\"");
        let matcher = Matcher::new(&ScopeTree::new([a, b, c], None).unwrap()).unwrap();
        assert_eq!(matcher.match_message(&facts("x.dev")).scopes.len(), 2);
        assert_eq!(matcher.match_message(&facts("c.dev")).scopes.len(), 3);
    }
    #[test]
    fn parent_failure_short_circuits_expensive_descendants() {
        let parent = scope(None, "subject == \"never\"");
        let child = scope(Some(parent.id), "text contains \"expensive\"");
        let matcher = Matcher::new(&ScopeTree::new([parent, child], None).unwrap()).unwrap();
        let result = matcher.match_message(&facts("a.dev"));
        assert!(result.scopes.is_empty());
        assert_eq!(result.stats.predicate_evaluations, 1);
    }
    #[test]
    fn set_deltas() {
        let old = BTreeSet::from([MessageSeq(1), MessageSeq(2)]);
        let new = BTreeSet::from([MessageSeq(2), MessageSeq(3)]);
        assert_eq!(
            MatchDelta::between(&old, &new),
            MatchDelta {
                added: BTreeSet::from([MessageSeq(3)]),
                removed: BTreeSet::from([MessageSeq(1)])
            }
        );
    }
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
            scopes.extend(sources.iter().map(|source| scope(Some(parent.id), source)));
        }
        let tree = ScopeTree::new(scopes, None).unwrap();
        let matcher = Matcher::new(&tree).unwrap();
        let queries: HashMap<_, _> = tree
            .scopes()
            .map(|scope| (scope.id, compile(&scope.filter).unwrap()))
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
                            queries[&scope.id].evaluate(&message)
                                && tree
                                    .ancestors(scope.id)
                                    .unwrap()
                                    .iter()
                                    .all(|parent| queries[parent].evaluate(&message))
                        })
                        .map(|scope| scope.id)
                        .collect();
                    let actual: BTreeSet<_> = matcher
                        .match_message(&message)
                        .scopes
                        .into_iter()
                        .map(|(id, _)| id)
                        .collect();
                    assert_eq!(
                        actual, expected,
                        "from={from}, to={to:?}, subject={subject}"
                    );
                }
            }
        }
    }
    #[test]
    fn deep_inheritance_evaluates_without_recursion_or_duplicate_predicates() {
        let root = scope(None, "from.domain == 'a.dev'");
        let mut parent = root.id;
        let mut scopes = vec![root];
        for _ in 0..1500 {
            let child = scope(Some(parent), "subject contains 'hello'");
            parent = child.id;
            scopes.push(child);
        }
        let matcher = Matcher::new(&ScopeTree::new(scopes, None).unwrap()).unwrap();
        let result = matcher.match_message(&facts("a.dev"));
        assert_eq!(result.scopes.len(), 1501);
        assert_eq!(result.stats.predicate_evaluations, 2);
        assert!(matcher.match_message(&facts("other.dev")).scopes.is_empty());
    }
}
