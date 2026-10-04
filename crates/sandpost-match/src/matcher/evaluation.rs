//! Shared expression interning and sparse, iterative per-message evaluation.
use super::{EvaluationNode, MatchStatistics, Matcher, NodeIdentifier};
use sandpost_core::MessageFacts;
use sandpost_query::Expression;
use std::collections::HashMap;

impl Matcher {
    /// Intern the canonical expression recursively, sharing equivalent child nodes.
    pub(super) fn intern_expression(&mut self, expression: &Expression) -> NodeIdentifier {
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
    pub(super) fn intern(&mut self, mut node: EvaluationNode) -> NodeIdentifier {
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

    /// Explicit evaluation stack supports arbitrarily deep scope hierarchies.
    pub(super) fn evaluate(
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
}
