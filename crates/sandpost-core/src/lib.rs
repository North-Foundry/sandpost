//! Infrastructure-independent domain models. Email normalization belongs to ingest.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

macro_rules! opaque_identifier {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);
        impl $name {
            /// Generate a fresh random UUID identifier.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }
        impl Default for $name {
            /// Create a fresh identifier using the same random generation as `new`.
            fn default() -> Self {
                Self::new()
            }
        }
        impl std::fmt::Display for $name {
            /// Write the identifier in the canonical UUID display format.
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(formatter)
            }
        }
        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            /// Parse a UUID string and preserve the parser error for invalid input.
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                value.parse().map(Self)
            }
        }
    };
}
opaque_identifier!(MessageIdentifier);
opaque_identifier!(ScopeIdentifier);
opaque_identifier!(UserIdentifier);
opaque_identifier!(InboxIdentifier);

/// Internal SQLite AUTOINCREMENT key; opaque UUIDs are used in public APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MessageSequence(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mailbox {
    pub address: String,
    pub domain: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageFacts {
    pub envelope_from: Option<Mailbox>,
    pub envelope_to: Vec<Mailbox>,
    pub from: Vec<Mailbox>,
    pub to: Vec<Mailbox>,
    #[serde(rename = "cc")]
    pub carbon_copy: Vec<Mailbox>,
    pub subject: String,
    pub text: String,
    #[serde(rename = "html")]
    pub markup_body: String,
    #[serde(rename = "message_id")]
    pub message_identifier: Option<String>,
    /// Unix seconds, UTC. DSL timestamp values use this same unit.
    pub received_at: i64,
    pub size: u64,
    pub attachment_count: u64,
    /// Lowercase header names; duplicate values are preserved.
    pub headers: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub filename: Option<String>,
    pub content_type: String,
    pub size: u64,
    /// SHA-256 reference for a future content-addressed filesystem store.
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    #[serde(rename = "id")]
    pub identifier: MessageIdentifier,
    pub facts: MessageFacts,
    #[serde(rename = "raw_mime")]
    pub raw_message: Vec<u8>,
    pub attachments: Vec<Attachment>,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    #[serde(rename = "id")]
    pub identifier: UserIdentifier,
    pub name: String,
    pub personal_filter: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    #[serde(rename = "admin")]
    Administrator,
    Member,
    Viewer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Membership {
    #[serde(rename = "user_id")]
    pub user_identifier: UserIdentifier,
    #[serde(rename = "scope_id")]
    pub scope_identifier: ScopeIdentifier,
    pub role: Role,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inbox {
    #[serde(rename = "id")]
    pub identifier: InboxIdentifier,
    #[serde(rename = "user_id")]
    pub user_identifier: UserIdentifier,
    pub name: String,
    pub filter: String,
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

#[cfg(test)]
mod tests {
    use super::*;
    /// Create a scope fixture with an optional parent and initial policy version.
    fn scope(parent: Option<ScopeIdentifier>) -> Scope {
        Scope {
            identifier: ScopeIdentifier::new(),
            parent,
            name: "scope".into(),
            description: None,
            filter: String::new(),
            position: 0,
            policy_version: 1,
        }
    }
    /// Verify traversal order and that invalid moves leave the original tree intact.
    #[test]
    fn traversal_and_atomic_moves() {
        let root_scope = scope(None);
        let child_scope = scope(Some(root_scope.identifier));
        let grandchild_scope = scope(Some(child_scope.identifier));
        let destination_scope = scope(None);
        let mut tree = ScopeTree::new(
            [
                root_scope.clone(),
                child_scope.clone(),
                grandchild_scope.clone(),
                destination_scope.clone(),
            ],
            Some(2),
        )
        .unwrap();
        assert_eq!(
            tree.ancestors(grandchild_scope.identifier).unwrap(),
            vec![child_scope.identifier, root_scope.identifier]
        );
        assert_eq!(
            tree.subtree(child_scope.identifier).unwrap(),
            vec![child_scope.identifier, grandchild_scope.identifier]
        );
        assert_eq!(
            tree.move_scope(root_scope.identifier, Some(grandchild_scope.identifier)),
            Err(TreeError::Cycle)
        );
        assert_eq!(tree.get(root_scope.identifier).unwrap().parent, None);
        assert_eq!(
            tree.move_scope(child_scope.identifier, Some(destination_scope.identifier))
                .unwrap(),
            vec![child_scope.identifier, grandchild_scope.identifier]
        );
        assert_eq!(tree.children(root_scope.identifier), &[]);
        assert_eq!(
            tree.ancestors(grandchild_scope.identifier).unwrap(),
            vec![child_scope.identifier, destination_scope.identifier]
        );
    }
    /// Reject unknown parents, excessive depth, and duplicate scope identifiers.
    #[test]
    fn topology_validation() {
        let root_scope = scope(None);
        let child_scope = scope(Some(root_scope.identifier));
        assert!(matches!(
            ScopeTree::new([child_scope.clone()], None),
            Err(TreeError::Unknown(_))
        ));
        assert!(matches!(
            ScopeTree::new([root_scope.clone(), child_scope], Some(0)),
            Err(TreeError::Depth(0))
        ));
        assert!(matches!(
            ScopeTree::new([root_scope.clone(), root_scope], None),
            Err(TreeError::Duplicate(_))
        ));
    }
    /// Verify filter edits advance versions only in the affected subtree.
    #[test]
    fn policy_invalidation_versions_only_the_affected_subtree() {
        let root_scope = scope(None);
        let child_scope = scope(Some(root_scope.identifier));
        let unrelated_scope = scope(None);
        let mut tree = ScopeTree::new(
            [
                root_scope.clone(),
                child_scope.clone(),
                unrelated_scope.clone(),
            ],
            None,
        )
        .unwrap();
        assert_eq!(
            tree.set_filter(root_scope.identifier, "false".into())
                .unwrap(),
            vec![root_scope.identifier, child_scope.identifier]
        );
        assert_eq!(tree.get(root_scope.identifier).unwrap().policy_version, 2);
        assert_eq!(tree.get(child_scope.identifier).unwrap().policy_version, 2);
        assert_eq!(
            tree.get(unrelated_scope.identifier).unwrap().policy_version,
            1
        );
    }
    /// Reject disconnected cycles and traverse deep trees without recursion.
    #[test]
    fn disconnected_cycles_and_deep_trees() {
        let root = scope(None);
        let mut first_cycle_scope = scope(None);
        let second_cycle_scope = scope(Some(first_cycle_scope.identifier));
        first_cycle_scope.parent = Some(second_cycle_scope.identifier);
        assert!(matches!(
            ScopeTree::new([root.clone(), first_cycle_scope, second_cycle_scope], None),
            Err(TreeError::Cycle)
        ));
        let mut scopes = vec![root.clone()];
        let mut parent = root.identifier;
        for _ in 0..2000 {
            let next = scope(Some(parent));
            parent = next.identifier;
            scopes.push(next);
        }
        let tree = ScopeTree::new(scopes, None).unwrap();
        assert_eq!(tree.ancestors(parent).unwrap().len(), 2000);
        assert_eq!(tree.subtree(root.identifier).unwrap().len(), 2001);
    }
}
