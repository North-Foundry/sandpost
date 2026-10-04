//! Scope policies and validated indexed hierarchy traversal and edits.
use crate::ScopeIdentifier;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scope {
    #[serde(rename = "id")]
    pub identifier: ScopeIdentifier,
    pub parent: Option<ScopeIdentifier>,
    pub name: String,
    pub description: Option<String>,
    /// Empty source means true. Compiled expressions live outside the domain crate.
    pub filter: String,
    pub position: i64,
    pub policy_version: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TreeError {
    #[error("unknown scope {0}")]
    Unknown(ScopeIdentifier),
    #[error("scope hierarchy contains a cycle")]
    Cycle,
    #[error("scope hierarchy exceeds maximum depth {0}")]
    Depth(usize),
    #[error("duplicate scope {0}")]
    Duplicate(ScopeIdentifier),
    #[error("scope {0} policy version is exhausted")]
    VersionOverflow(ScopeIdentifier),
}

/// Indexed adjacency tree. Ancestors are O(depth); traversal is O(subtree size).
/// No recursion: arbitrary configured depth does not consume the thread stack.
#[derive(Debug, Clone, Default)]
pub struct ScopeTree {
    scopes: HashMap<ScopeIdentifier, Scope>,
    children: HashMap<Option<ScopeIdentifier>, Vec<ScopeIdentifier>>,
    maximum_depth: Option<usize>,
}

impl ScopeTree {
    /// Index scopes, validate parents, cycles and depth, and order siblings deterministically.
    pub fn new(
        scopes: impl IntoIterator<Item = Scope>,
        maximum_depth: Option<usize>,
    ) -> Result<Self, TreeError> {
        let mut tree = Self {
            maximum_depth,
            ..Self::default()
        };
        for scope in scopes {
            let identifier = scope.identifier;
            if tree.scopes.insert(identifier, scope).is_some() {
                return Err(TreeError::Duplicate(identifier));
            }
        }
        for scope in tree.scopes.values() {
            if let Some(parent) = scope.parent
                && !tree.scopes.contains_key(&parent)
            {
                return Err(TreeError::Unknown(parent));
            }
            tree.children
                .entry(scope.parent)
                .or_default()
                .push(scope.identifier);
        }
        for scope_identifiers in tree.children.values_mut() {
            scope_identifiers
                .sort_by_key(|identifier| (tree.scopes[identifier].position, *identifier));
        }
        // Each node has exactly one parent: any node unreachable from a root
        // belongs to a cyclic component. Validate once, not once per ancestor path.
        let mut stack: Vec<_> = tree
            .roots()
            .iter()
            .map(|identifier| (*identifier, 0usize))
            .collect();
        let mut visited = 0;
        while let Some((identifier, depth)) = stack.pop() {
            if let Some(maximum_allowed_depth) = maximum_depth
                && depth > maximum_allowed_depth
            {
                return Err(TreeError::Depth(maximum_allowed_depth));
            }
            visited += 1;
            stack.extend(
                tree.children(identifier)
                    .iter()
                    .map(|identifier| (*identifier, depth + 1)),
            );
        }
        if visited != tree.scopes.len() {
            return Err(TreeError::Cycle);
        }
        Ok(tree)
    }
    /// Return the scope with this identifier, or none when it is unknown.
    pub fn get(&self, identifier: ScopeIdentifier) -> Option<&Scope> {
        self.scopes.get(&identifier)
    }
    /// Iterate over all scopes; hash-map iteration order is unspecified.
    pub fn scopes(&self) -> impl Iterator<Item = &Scope> {
        self.scopes.values()
    }
    /// Return root identifiers ordered by position, then identifier.
    pub fn roots(&self) -> &[ScopeIdentifier] {
        self.children.get(&None).map(Vec::as_slice).unwrap_or(&[])
    }
    /// Return ordered children, or an empty slice for an unknown or childless scope.
    pub fn children(&self, identifier: ScopeIdentifier) -> &[ScopeIdentifier] {
        self.children
            .get(&Some(identifier))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    /// Closest parent first; excludes self. Roots have depth zero.
    pub fn ancestors(
        &self,
        identifier: ScopeIdentifier,
    ) -> Result<Vec<ScopeIdentifier>, TreeError> {
        let mut seen = HashSet::from([identifier]);
        let mut result = Vec::new();
        let mut parent = self
            .scopes
            .get(&identifier)
            .ok_or(TreeError::Unknown(identifier))?
            .parent;
        while let Some(identifier) = parent {
            if !seen.insert(identifier) {
                return Err(TreeError::Cycle);
            }
            result.push(identifier);
            parent = self
                .scopes
                .get(&identifier)
                .ok_or(TreeError::Unknown(identifier))?
                .parent;
        }
        Ok(result)
    }
    /// Preorder, including the selected root. This is the invalidation boundary.
    pub fn subtree(&self, identifier: ScopeIdentifier) -> Result<Vec<ScopeIdentifier>, TreeError> {
        if !self.scopes.contains_key(&identifier) {
            return Err(TreeError::Unknown(identifier));
        }
        let mut stack = vec![identifier];
        let mut result = Vec::new();
        while let Some(identifier) = stack.pop() {
            result.push(identifier);
            stack.extend(self.children(identifier).iter().rev().copied());
        }
        Ok(result)
    }
    /// Validate the complete prospective topology before publishing the move.
    pub fn move_scope(
        &mut self,
        identifier: ScopeIdentifier,
        parent: Option<ScopeIdentifier>,
    ) -> Result<Vec<ScopeIdentifier>, TreeError> {
        let affected = self.subtree(identifier)?;
        let mut scopes = self.scopes.clone();
        scopes
            .get_mut(&identifier)
            .ok_or(TreeError::Unknown(identifier))?
            .parent = parent;
        for identifier in &affected {
            let scope = scopes
                .get_mut(identifier)
                .ok_or(TreeError::Unknown(*identifier))?;
            scope.policy_version = scope
                .policy_version
                .checked_add(1)
                .ok_or(TreeError::VersionOverflow(*identifier))?;
        }
        let next = Self::new(scopes.into_values(), self.maximum_depth)?;
        *self = next;
        Ok(affected)
    }
    /// Return precisely the subtree that needs a new compiled policy and
    /// materialization. Query validation occurs before calling this domain edit.
    pub fn set_filter(
        &mut self,
        identifier: ScopeIdentifier,
        filter: String,
    ) -> Result<Vec<ScopeIdentifier>, TreeError> {
        let affected = self.subtree(identifier)?;
        for scope_identifier in &affected {
            let scope = self
                .scopes
                .get(scope_identifier)
                .ok_or(TreeError::Unknown(*scope_identifier))?;
            if scope.policy_version == u64::MAX {
                return Err(TreeError::VersionOverflow(*scope_identifier));
            }
        }
        self.scopes
            .get_mut(&identifier)
            .ok_or(TreeError::Unknown(identifier))?
            .filter = filter;
        for scope_identifier in &affected {
            let scope = self
                .scopes
                .get_mut(scope_identifier)
                .ok_or(TreeError::Unknown(*scope_identifier))?;
            scope.policy_version += 1;
        }
        Ok(affected)
    }
}
