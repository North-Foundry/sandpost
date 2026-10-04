//! Immutable policy snapshot construction, indexed matching, and result statistics.
use crate::candidates::{CandidateAnchor, anchor_cover, intersect_cover};
use sandpost_core::{MessageFacts, ScopeIdentifier, ScopeTree, TreeError};
use sandpost_query::{Field, Predicate, QueryError, compile};
use std::collections::{BTreeSet, HashMap, HashSet};

mod evaluation;

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

    /// Return the number of distinct interned nodes in this policy snapshot.
    pub fn shared_node_count(&self) -> usize {
        self.nodes.len()
    }
    /// Return the number of scopes without a safe positive index anchor.
    pub fn fallback_scope_count(&self) -> usize {
        self.fallback.len()
    }
}
