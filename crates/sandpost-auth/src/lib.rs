#![doc = include_str!("../README.md")]
use sandpost_core::{Membership, MessageFacts, Role, ScopeIdentifier, ScopeTree, UserIdentifier};
use sandpost_query::CompiledQuery;
use std::collections::HashSet;

/// Membership visibility is a union, followed by a personal restriction.
/// Empty membership fails closed. Callers supply only current materializations.
pub fn can_view(
    user: UserIdentifier,
    memberships: &[Membership],
    matches: &HashSet<ScopeIdentifier>,
    personal_filter: Option<&CompiledQuery>,
    facts: &MessageFacts,
) -> bool {
    memberships.iter().any(|membership| {
        membership.user_identifier == user && matches.contains(&membership.scope_identifier)
    }) && personal_filter.is_none_or(|filter| filter.evaluate(facts))
}

/// Owner or administrator membership can manage its scope and descendants. Viewer/member
/// has no administrative grant, even if its mail visibility matches.
pub fn can_manage(
    user: UserIdentifier,
    target: ScopeIdentifier,
    memberships: &[Membership],
    tree: &ScopeTree,
) -> bool {
    let Ok(ancestors) = tree.ancestors(target) else {
        return false;
    };
    memberships.iter().any(|membership| {
        membership.user_identifier == user
            && matches!(membership.role, Role::Owner | Role::Administrator)
            && (membership.scope_identifier == target
                || ancestors.contains(&membership.scope_identifier))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandpost_core::Scope;
    /// Verify membership unions, personal restrictions, and administrative inheritance.
    #[test]
    fn membership_union_personal_intersection_and_roles_are_separate() {
        let user = UserIdentifier::new();
        let first_scope = ScopeIdentifier::new();
        let second_scope = ScopeIdentifier::new();
        let memberships = vec![
            Membership {
                user_identifier: user,
                scope_identifier: first_scope,
                role: Role::Viewer,
            },
            Membership {
                user_identifier: user,
                scope_identifier: second_scope,
                role: Role::Administrator,
            },
        ];
        let facts = MessageFacts {
            subject: "reset".into(),
            ..MessageFacts::default()
        };
        assert!(can_view(
            user,
            &memberships,
            &HashSet::from([first_scope]),
            None,
            &facts
        ));
        assert!(can_view(
            user,
            &memberships,
            &HashSet::from([second_scope]),
            None,
            &facts
        ));
        assert!(!can_view(
            user,
            &[],
            &HashSet::from([first_scope]),
            None,
            &facts
        ));
        let personal = sandpost_query::compile("subject == \"other\"").unwrap();
        assert!(!can_view(
            user,
            &memberships,
            &HashSet::from([first_scope]),
            Some(&personal),
            &facts
        ));
        let build_scope = |identifier, parent| Scope {
            identifier,
            parent,
            name: "test".into(),
            description: None,
            filter: String::new(),
            position: 0,
            policy_version: 1,
        };
        let child = ScopeIdentifier::new();
        let tree = ScopeTree::new(
            [
                build_scope(first_scope, None),
                build_scope(second_scope, None),
                build_scope(child, Some(second_scope)),
            ],
            None,
        )
        .unwrap();
        assert!(!can_manage(user, first_scope, &memberships, &tree));
        assert!(can_manage(user, child, &memberships, &tree));
        assert!(!can_manage(
            user,
            ScopeIdentifier::new(),
            &memberships,
            &tree
        ));
    }
}
