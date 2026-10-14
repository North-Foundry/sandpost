//! Single composition boundary for search, direct reads, attachments, and events.
use crate::{ApplicationError, QueryOptions};
use sandpost_core::{GlobalRole, MailAccess, MessageIdentifier, ScopeTree, User, UserIdentifier};
use sandpost_query::Expression;
use sandpost_search::MessageQuery;
use sandpost_storage::Storage;
use std::collections::HashSet;
use std::sync::Arc;

/// Load authoritative user state, refusing stale or unknown principal identities.
pub(crate) async fn user(
    storage: &Arc<dyn Storage>,
    user: UserIdentifier,
) -> Result<User, ApplicationError> {
    storage
        .get_user(user)
        .await?
        .ok_or(ApplicationError::Forbidden)
}

/// Enforce an explicit action independently of mail visibility.
pub(crate) fn require_action(allowed: bool) -> Result<(), ApplicationError> {
    if allowed {
        Ok(())
    } else {
        Err(ApplicationError::Forbidden)
    }
}

/// Parse the sole canonical query language and retain precise validation diagnostics.
pub(crate) fn expression(source: &str) -> Result<Expression, ApplicationError> {
    sandpost_query::compile(source)
        .map(|query| query.into_expression())
        .map_err(|error| ApplicationError::InvalidInput(error.to_string()))
}

/// Compose mail access, scope unions, personal restrictions, Views, and ad hoc queries.
///
/// Owners read all mail without personal restrictions. Every other account reads all mail or,
/// with scoped access, the union of its assigned scopes (each combined with its ancestors); a
/// personal filter further restricts either.
pub(crate) async fn compose(
    storage: &Arc<dyn Storage>,
    user: UserIdentifier,
    options: &QueryOptions,
    message: Option<MessageIdentifier>,
) -> Result<MessageQuery, ApplicationError> {
    let account = self::user(storage, user).await?;
    let full_authority = account.global_role == GlobalRole::Owner;
    let authorization = if full_authority {
        None
    } else {
        mail_authorization(storage, &account).await?
    };
    let mut filters = Vec::new();
    if !full_authority && let Some(personal) = &account.personal_filter {
        filters.push(expression(personal)?);
    }
    if let Some(identifier) = options.view_identifier {
        let view = storage
            .get_view(identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        if view.owner_identifier.is_some_and(|owner| owner != user) && !full_authority {
            return Err(ApplicationError::NotFound);
        }
        filters.push(expression(&view.filter)?);
    }
    if let Some(source) = &options.query {
        filters.push(expression(source)?);
    }
    Ok(MessageQuery {
        authorization,
        filter: Expression::And(filters),
        message_identifier: message,
        before: options.before,
        limit: options.limit,
    })
}

/// Resolve which mail an account may read: `None` for all mail, otherwise the union of its
/// assigned scopes with inherited filters (`False` when nothing is assigned).
async fn mail_authorization(
    storage: &Arc<dyn Storage>,
    account: &User,
) -> Result<Option<Expression>, ApplicationError> {
    if account.mail_access == MailAccess::All {
        return Ok(None);
    }
    let tree = ScopeTree::new(storage.list_scopes().await?, None)
        .map_err(|error| ApplicationError::Unavailable(error.to_string()))?;
    let assigned: HashSet<_> = storage
        .list_user_scopes(account.identifier)
        .await?
        .into_iter()
        .collect();
    let mut grants = Vec::new();
    for scope_identifier in &assigned {
        let Some(scope) = tree.get(*scope_identifier) else {
            continue;
        };
        let mut effective = vec![expression(&scope.filter)?];
        for ancestor in tree
            .ancestors(*scope_identifier)
            .map_err(|error| ApplicationError::Unavailable(error.to_string()))?
        {
            let parent = tree.get(ancestor).ok_or(ApplicationError::Forbidden)?;
            effective.push(expression(&parent.filter)?);
        }
        grants.push(Expression::And(effective));
    }
    Ok(Some(if grants.is_empty() {
        Expression::False
    } else {
        Expression::Or(grants)
    }))
}

/// Use the same structured plan against authoritative current SQL facts for direct reads.
pub(crate) async fn is_visible(
    storage: &Arc<dyn Storage>,
    user: UserIdentifier,
    identifier: MessageIdentifier,
) -> Result<bool, ApplicationError> {
    let query = compose(storage, user, &QueryOptions::default(), Some(identifier)).await?;
    let Some((facts, _)) = storage.get_user_message_metadata(user, identifier).await? else {
        return Ok(false);
    };
    Ok(matches_snapshot(&query, &facts))
}

/// Combine authorization and requested filters for authoritative SQL operations.
pub(crate) fn storage_filter(query: &MessageQuery) -> Expression {
    Expression::And(vec![
        query.authorization.clone().unwrap_or(Expression::True),
        query.filter.clone(),
    ])
}

/// Evaluate one composed plan against facts read from one SQL snapshot.
pub(crate) fn matches_snapshot(query: &MessageQuery, facts: &sandpost_core::MessageFacts) -> bool {
    query
        .authorization
        .as_ref()
        .is_none_or(|authorization| authorization.evaluate(facts))
        && query.filter.evaluate(facts)
}

/// Authorize the exact snapshot a caller will return without fetching another message version.
pub(crate) async fn require_snapshot(
    storage: &Arc<dyn Storage>,
    user: UserIdentifier,
    facts: &sandpost_core::MessageFacts,
) -> Result<(), ApplicationError> {
    let query = compose(storage, user, &QueryOptions::default(), None).await?;
    if matches_snapshot(&query, facts) {
        Ok(())
    } else {
        Err(ApplicationError::NotFound)
    }
}

/// Validate a prospective hierarchy before publishing a query-definition edit.
pub(crate) async fn validate_scope_tree(
    storage: &Arc<dyn Storage>,
    replacement: &sandpost_core::Scope,
    maximum_depth: Option<usize>,
) -> Result<(), ApplicationError> {
    expression(&replacement.filter)?;
    let mut scopes = storage.list_scopes().await?;
    scopes.retain(|scope| scope.identifier != replacement.identifier);
    scopes.push(replacement.clone());
    ScopeTree::new(scopes, maximum_depth)
        .map_err(|error| ApplicationError::InvalidInput(error.to_string()))?;
    Ok(())
}
