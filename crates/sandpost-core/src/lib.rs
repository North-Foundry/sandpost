//! Infrastructure-independent domain models. Email normalization belongs to ingest.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);
        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
        impl std::str::FromStr for $name {
            type Err = uuid::Error;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                value.parse().map(Self)
            }
        }
    };
}
opaque_id!(MessageId);
opaque_id!(ScopeId);
opaque_id!(UserId);
opaque_id!(InboxId);

/// Internal SQLite AUTOINCREMENT key; opaque UUIDs are used in public APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MessageSeq(pub u64);

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
    pub cc: Vec<Mailbox>,
    pub subject: String,
    pub text: String,
    pub html: String,
    pub message_id: Option<String>,
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
    pub id: MessageId,
    pub facts: MessageFacts,
    pub raw_mime: Vec<u8>,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scope {
    pub id: ScopeId,
    pub parent: Option<ScopeId>,
    pub name: String,
    pub description: Option<String>,
    /// Empty source means true. Compiled expressions live outside the domain crate.
    pub filter: String,
    pub position: i64,
    pub policy_version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: UserId,
    pub name: String,
    pub personal_filter: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Admin,
    Member,
    Viewer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Membership {
    pub user_id: UserId,
    pub scope_id: ScopeId,
    pub role: Role,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inbox {
    pub id: InboxId,
    pub user_id: UserId,
    pub name: String,
    pub filter: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TreeError {
    #[error("unknown scope {0}")]
    Unknown(ScopeId),
    #[error("scope hierarchy contains a cycle")]
    Cycle,
    #[error("scope hierarchy exceeds maximum depth {0}")]
    Depth(usize),
    #[error("duplicate scope {0}")]
    Duplicate(ScopeId),
    #[error("scope {0} policy version is exhausted")]
    VersionOverflow(ScopeId),
}

/// Indexed adjacency tree. Ancestors are O(depth); traversal is O(subtree size).
/// No recursion: arbitrary configured depth does not consume the thread stack.
#[derive(Debug, Clone, Default)]
pub struct ScopeTree {
    scopes: HashMap<ScopeId, Scope>,
    children: HashMap<Option<ScopeId>, Vec<ScopeId>>,
    max_depth: Option<usize>,
}

impl ScopeTree {
    pub fn new(
        scopes: impl IntoIterator<Item = Scope>,
        max_depth: Option<usize>,
    ) -> Result<Self, TreeError> {
        let mut tree = Self {
            max_depth,
            ..Self::default()
        };
        for scope in scopes {
            let id = scope.id;
            if tree.scopes.insert(id, scope).is_some() {
                return Err(TreeError::Duplicate(id));
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
                .push(scope.id);
        }
        for ids in tree.children.values_mut() {
            ids.sort_by_key(|id| (tree.scopes[id].position, *id));
        }
        // Each node has exactly one parent: any node unreachable from a root
        // belongs to a cyclic component. Validate once, not once per ancestor path.
        let mut stack: Vec<_> = tree.roots().iter().map(|id| (*id, 0usize)).collect();
        let mut visited = 0;
        while let Some((id, depth)) = stack.pop() {
            if let Some(max) = max_depth
                && depth > max
            {
                return Err(TreeError::Depth(max));
            }
            visited += 1;
            stack.extend(tree.children(id).iter().map(|id| (*id, depth + 1)));
        }
        if visited != tree.scopes.len() {
            return Err(TreeError::Cycle);
        }
        Ok(tree)
    }
    pub fn get(&self, id: ScopeId) -> Option<&Scope> {
        self.scopes.get(&id)
    }
    pub fn scopes(&self) -> impl Iterator<Item = &Scope> {
        self.scopes.values()
    }
    pub fn roots(&self) -> &[ScopeId] {
        self.children.get(&None).map(Vec::as_slice).unwrap_or(&[])
    }
    pub fn children(&self, id: ScopeId) -> &[ScopeId] {
        self.children
            .get(&Some(id))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    /// Closest parent first; excludes self. Roots have depth zero.
    pub fn ancestors(&self, id: ScopeId) -> Result<Vec<ScopeId>, TreeError> {
        let mut seen = HashSet::from([id]);
        let mut result = Vec::new();
        let mut parent = self.scopes.get(&id).ok_or(TreeError::Unknown(id))?.parent;
        while let Some(id) = parent {
            if !seen.insert(id) {
                return Err(TreeError::Cycle);
            }
            result.push(id);
            parent = self.scopes.get(&id).ok_or(TreeError::Unknown(id))?.parent;
        }
        Ok(result)
    }
    /// Preorder, including the selected root. This is the invalidation boundary.
    pub fn subtree(&self, id: ScopeId) -> Result<Vec<ScopeId>, TreeError> {
        if !self.scopes.contains_key(&id) {
            return Err(TreeError::Unknown(id));
        }
        let mut stack = vec![id];
        let mut result = Vec::new();
        while let Some(id) = stack.pop() {
            result.push(id);
            stack.extend(self.children(id).iter().rev().copied());
        }
        Ok(result)
    }
    /// Validate the complete prospective topology before publishing the move.
    pub fn move_scope(
        &mut self,
        id: ScopeId,
        parent: Option<ScopeId>,
    ) -> Result<Vec<ScopeId>, TreeError> {
        let affected = self.subtree(id)?;
        let mut scopes = self.scopes.clone();
        scopes.get_mut(&id).ok_or(TreeError::Unknown(id))?.parent = parent;
        for id in &affected {
            let scope = scopes.get_mut(id).ok_or(TreeError::Unknown(*id))?;
            scope.policy_version = scope
                .policy_version
                .checked_add(1)
                .ok_or(TreeError::VersionOverflow(*id))?;
        }
        let next = Self::new(scopes.into_values(), self.max_depth)?;
        *self = next;
        Ok(affected)
    }
    /// Return precisely the subtree that needs a new compiled policy and
    /// materialization. Query validation occurs before calling this domain edit.
    pub fn set_filter(&mut self, id: ScopeId, filter: String) -> Result<Vec<ScopeId>, TreeError> {
        let affected = self.subtree(id)?;
        for scope_id in &affected {
            let scope = self
                .scopes
                .get(scope_id)
                .ok_or(TreeError::Unknown(*scope_id))?;
            if scope.policy_version == u64::MAX {
                return Err(TreeError::VersionOverflow(*scope_id));
            }
        }
        self.scopes
            .get_mut(&id)
            .ok_or(TreeError::Unknown(id))?
            .filter = filter;
        for scope_id in &affected {
            let scope = self
                .scopes
                .get_mut(scope_id)
                .ok_or(TreeError::Unknown(*scope_id))?;
            scope.policy_version += 1;
        }
        Ok(affected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope(parent: Option<ScopeId>) -> Scope {
        Scope {
            id: ScopeId::new(),
            parent,
            name: "scope".into(),
            description: None,
            filter: String::new(),
            position: 0,
            policy_version: 1,
        }
    }
    #[test]
    fn traversal_and_atomic_moves() {
        let a = scope(None);
        let b = scope(Some(a.id));
        let c = scope(Some(b.id));
        let other = scope(None);
        let mut tree =
            ScopeTree::new([a.clone(), b.clone(), c.clone(), other.clone()], Some(2)).unwrap();
        assert_eq!(tree.ancestors(c.id).unwrap(), vec![b.id, a.id]);
        assert_eq!(tree.subtree(b.id).unwrap(), vec![b.id, c.id]);
        assert_eq!(tree.move_scope(a.id, Some(c.id)), Err(TreeError::Cycle));
        assert_eq!(tree.get(a.id).unwrap().parent, None);
        assert_eq!(
            tree.move_scope(b.id, Some(other.id)).unwrap(),
            vec![b.id, c.id]
        );
        assert_eq!(tree.children(a.id), &[]);
        assert_eq!(tree.ancestors(c.id).unwrap(), vec![b.id, other.id]);
    }
    #[test]
    fn topology_validation() {
        let a = scope(None);
        let b = scope(Some(a.id));
        assert!(matches!(
            ScopeTree::new([b.clone()], None),
            Err(TreeError::Unknown(_))
        ));
        assert!(matches!(
            ScopeTree::new([a.clone(), b], Some(0)),
            Err(TreeError::Depth(0))
        ));
        assert!(matches!(
            ScopeTree::new([a.clone(), a], None),
            Err(TreeError::Duplicate(_))
        ));
    }
    #[test]
    fn policy_invalidation_versions_only_the_affected_subtree() {
        let a = scope(None);
        let b = scope(Some(a.id));
        let c = scope(None);
        let mut tree = ScopeTree::new([a.clone(), b.clone(), c.clone()], None).unwrap();
        assert_eq!(
            tree.set_filter(a.id, "false".into()).unwrap(),
            vec![a.id, b.id]
        );
        assert_eq!(tree.get(a.id).unwrap().policy_version, 2);
        assert_eq!(tree.get(b.id).unwrap().policy_version, 2);
        assert_eq!(tree.get(c.id).unwrap().policy_version, 1);
    }
    #[test]
    fn disconnected_cycles_and_deep_trees() {
        let root = scope(None);
        let mut a = scope(None);
        let b = scope(Some(a.id));
        a.parent = Some(b.id);
        assert!(matches!(
            ScopeTree::new([root.clone(), a, b], None),
            Err(TreeError::Cycle)
        ));
        let mut scopes = vec![root.clone()];
        let mut parent = root.id;
        for _ in 0..2000 {
            let next = scope(Some(parent));
            parent = next.id;
            scopes.push(next);
        }
        let tree = ScopeTree::new(scopes, None).unwrap();
        assert_eq!(tree.ancestors(parent).unwrap().len(), 2000);
        assert_eq!(tree.subtree(root.id).unwrap().len(), 2001);
    }
}
