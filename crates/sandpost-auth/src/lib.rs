//! Administrative permissions and mail visibility are deliberately separate.
use sandpost_core::{Membership, MessageFacts, Role, ScopeId, ScopeTree, UserId};
use sandpost_query::CompiledQuery;
use std::collections::HashSet;

/// Membership visibility is a union, followed by a personal restriction.
/// Empty membership fails closed. Callers supply only current materializations.
pub fn can_view(
    user: UserId,
    memberships: &[Membership],
    matches: &HashSet<ScopeId>,
    personal_filter: Option<&CompiledQuery>,
    facts: &MessageFacts,
) -> bool {
    memberships
        .iter()
        .any(|m| m.user_id == user && matches.contains(&m.scope_id))
        && personal_filter.is_none_or(|filter| filter.evaluate(facts))
}

/// Owner/admin membership can manage its scope and descendants. Viewer/member
/// has no administrative grant, even if its mail visibility matches.
pub fn can_manage(
    user: UserId,
    target: ScopeId,
    memberships: &[Membership],
    tree: &ScopeTree,
) -> bool {
    let Ok(ancestors) = tree.ancestors(target) else {
        return false;
    };
    memberships.iter().any(|m| {
        m.user_id == user
            && matches!(m.role, Role::Owner | Role::Admin)
            && (m.scope_id == target || ancestors.contains(&m.scope_id))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandpost_core::Scope;
    #[test]
    fn membership_union_personal_intersection_and_roles_are_separate() {
        let user = UserId::new();
        let a = ScopeId::new();
        let b = ScopeId::new();
        let memberships = vec![
            Membership {
                user_id: user,
                scope_id: a,
                role: Role::Viewer,
            },
            Membership {
                user_id: user,
                scope_id: b,
                role: Role::Admin,
            },
        ];
        let facts = MessageFacts {
            subject: "reset".into(),
            ..MessageFacts::default()
        };
        assert!(can_view(
            user,
            &memberships,
            &HashSet::from([a]),
            None,
            &facts
        ));
        assert!(can_view(
            user,
            &memberships,
            &HashSet::from([b]),
            None,
            &facts
        ));
        assert!(!can_view(user, &[], &HashSet::from([a]), None, &facts));
        let personal = sandpost_query::compile("subject == \"other\"").unwrap();
        assert!(!can_view(
            user,
            &memberships,
            &HashSet::from([a]),
            Some(&personal),
            &facts
        ));
        let scope = |id, parent| Scope {
            id,
            parent,
            name: "test".into(),
            description: None,
            filter: String::new(),
            position: 0,
            policy_version: 1,
        };
        let child = ScopeId::new();
        let tree = ScopeTree::new(
            [scope(a, None), scope(b, None), scope(child, Some(b))],
            None,
        )
        .unwrap();
        assert!(!can_manage(user, a, &memberships, &tree));
        assert!(can_manage(user, child, &memberships, &tree));
        assert!(!can_manage(user, ScopeId::new(), &memberships, &tree));
    }
}
