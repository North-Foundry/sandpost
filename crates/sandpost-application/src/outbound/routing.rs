//! Routing policy: which routing actions apply to a stored message and to which recipients.
//!
//! Scopes are query definitions, so a message belongs to every scope whose filter and ancestor
//! filters it satisfies. Routing uses only the most specific of those scopes: a matching scope
//! with a matching child yields to the child. Routing actions are explicit: an action applies to
//! mail whose most specific matching scopes include its own scope, and is never inherited by
//! child scopes. [`RoutingConfiguration::applicable_scopes`] is the single place where an
//! inheritance policy could later add ancestors without changing delivery.
use super::validation::recipient_allowed;
use crate::ApplicationError;
use sandpost_core::{
    MessageFacts, OutboundDeliveryIdentifier, OutboundDeliveryStatus, OutboundFailure,
    OutboundFailureKind, OutboundRecipientStatus, RoutingAction, RoutingActionKind,
    ScopeIdentifier, ScopeTree, SmtpOutbound, SmtpOutboundIdentifier,
};
use sandpost_query::CompiledQuery;
use sandpost_storage::{NewOutboundDelivery, NewOutboundRecipient, Storage};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// More `Received` fields than this means the message is probably looping back through an SMTP
/// Outbound that delivers to Sandpost itself, so it is not routed again.
pub(crate) const MAXIMUM_RECEIVED_FIELDS: usize = 30;

/// A snapshot of the enabled routing configuration used to evaluate queued messages.
pub(crate) struct RoutingConfiguration {
    tree: ScopeTree,
    filters: HashMap<ScopeIdentifier, CompiledQuery>,
    actions: HashMap<ScopeIdentifier, Vec<RoutingAction>>,
    outbounds: HashMap<SmtpOutboundIdentifier, SmtpOutbound>,
}

/**
 * Build a policy failure.
 */
fn policy_failure(message: impl Into<String>) -> OutboundFailure {
    OutboundFailure {
        kind: OutboundFailureKind::Policy,
        message: message.into(),
        code: None,
    }
}

impl RoutingConfiguration {
    /**
     * Read scopes, enabled routing actions and enabled SMTP Outbounds.
     *
     * A scope whose stored filter no longer compiles matches nothing, so it can never widen
     * routing; persisted filters are validated when saved, so this is only a safeguard.
     */
    pub(crate) async fn load(storage: &Arc<dyn Storage>) -> Result<Self, ApplicationError> {
        let tree = ScopeTree::new(storage.list_scopes().await?, None)
            .map_err(|error| ApplicationError::Unavailable(error.to_string()))?;
        let mut filters = HashMap::new();
        for scope in tree.scopes() {
            match sandpost_query::compile(&scope.filter) {
                Ok(compiled) => {
                    filters.insert(scope.identifier, compiled);
                }
                Err(error) => {
                    tracing::warn!(scope = %scope.identifier, %error, "scope filter does not compile; it routes no mail");
                }
            }
        }
        let outbounds: HashMap<_, _> = storage
            .list_smtp_outbounds()
            .await?
            .into_iter()
            .filter(|outbound| outbound.enabled)
            .map(|outbound| (outbound.identifier, outbound))
            .collect();
        let mut actions: HashMap<ScopeIdentifier, Vec<RoutingAction>> = HashMap::new();
        for action in storage.list_routing_actions(None).await? {
            if action.enabled && outbounds.contains_key(&action.smtp_outbound_identifier) {
                actions
                    .entry(action.scope_identifier)
                    .or_default()
                    .push(action);
            }
        }
        Ok(Self {
            tree,
            filters,
            actions,
            outbounds,
        })
    }

    /**
     * Report whether any enabled action uses an enabled SMTP Outbound.
     */
    pub(crate) fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /**
     * Return the most specific scopes whose own and ancestor filters the message satisfies:
     * matching scopes without a matching child, in tree order.
     */
    pub(crate) fn matching_scopes(&self, facts: &MessageFacts) -> Vec<ScopeIdentifier> {
        let mut matched = HashSet::new();
        let mut resolved = Vec::new();
        let mut pending: Vec<ScopeIdentifier> = self.tree.roots().iter().rev().copied().collect();
        // Depth-first in tree order: a scope is visited only when its parent matched, so a
        // match always implies every ancestor matched too.
        let mut order = Vec::new();
        while let Some(scope) = pending.pop() {
            let satisfied = self
                .filters
                .get(&scope)
                .is_some_and(|filter| filter.evaluate(facts));
            if satisfied {
                matched.insert(scope);
                order.push(scope);
                pending.extend(self.tree.children(scope).iter().rev().copied());
            }
        }
        for scope in order {
            if !self
                .tree
                .children(scope)
                .iter()
                .any(|child| matched.contains(child))
            {
                resolved.push(scope);
            }
        }
        resolved
    }

    /**
     * Return the scopes whose routing actions apply to a message, given its most specific
     * matching scopes.
     *
     * Routing is explicit: only those scopes' own actions apply. An inheritance policy would add
     * ancestors here; nothing else in routing or delivery depends on this choice.
     */
    pub(crate) fn applicable_scopes(&self, matching: Vec<ScopeIdentifier>) -> Vec<ScopeIdentifier> {
        matching
    }

    /**
     * Decide the deliveries a stored message needs, one per applicable enabled action.
     *
     * Relay actions use the message's SMTP envelope recipients (which include Bcc recipients and
     * keep their original spelling), forward actions their configured recipients. Recipients the
     * SMTP Outbound's allowlist does not permit are recorded as failed without being attempted;
     * a delivery left without any permitted recipient, or a message that already passed through
     * too many servers, is recorded as failed so the decision stays visible.
     */
    pub(crate) fn deliveries(&self, facts: &MessageFacts, now: i64) -> Vec<NewOutboundDelivery> {
        let scopes = self.applicable_scopes(self.matching_scopes(facts));
        let looping = facts
            .headers
            .get("received")
            .is_some_and(|fields| fields.len() > MAXIMUM_RECEIVED_FIELDS);
        let mut deliveries = Vec::new();
        for scope in scopes {
            for action in self.actions.get(&scope).into_iter().flatten() {
                let Some(outbound) = self.outbounds.get(&action.smtp_outbound_identifier) else {
                    continue;
                };
                let addresses = match action.kind {
                    RoutingActionKind::Relay => {
                        let mut seen = HashSet::new();
                        facts
                            .envelope_to
                            .iter()
                            .map(|mailbox| mailbox.address.clone())
                            .filter(|address| seen.insert(address.to_ascii_lowercase()))
                            .collect::<Vec<_>>()
                    }
                    RoutingActionKind::Forward => action.forward_recipients.clone(),
                };
                let recipients: Vec<NewOutboundRecipient> = addresses
                    .into_iter()
                    .map(|address| {
                        if recipient_allowed(&address, &outbound.allowed_recipients) {
                            NewOutboundRecipient {
                                address,
                                status: OutboundRecipientStatus::Pending,
                                error: None,
                            }
                        } else {
                            NewOutboundRecipient {
                                address,
                                status: OutboundRecipientStatus::Failed,
                                error: Some(policy_failure(
                                    "the SMTP Outbound's recipient allowlist does not permit this recipient",
                                )),
                            }
                        }
                    })
                    .collect();
                let blocked = if looping {
                    Some(policy_failure(format!(
                        "the message already passed through more than {MAXIMUM_RECEIVED_FIELDS} servers; it is not routed again to avoid a mail loop"
                    )))
                } else if recipients.is_empty() {
                    Some(policy_failure("the message has no recipient to deliver to"))
                } else if recipients
                    .iter()
                    .all(|recipient| recipient.status == OutboundRecipientStatus::Failed)
                {
                    Some(policy_failure(
                        "the SMTP Outbound's recipient allowlist permits none of the recipients",
                    ))
                } else {
                    None
                };
                let recipients = match &blocked {
                    Some(failure) if looping => recipients
                        .into_iter()
                        .map(|recipient| NewOutboundRecipient {
                            status: OutboundRecipientStatus::Failed,
                            error: Some(failure.clone()),
                            ..recipient
                        })
                        .collect(),
                    _ => recipients,
                };
                deliveries.push(NewOutboundDelivery {
                    identifier: OutboundDeliveryIdentifier::new(),
                    scope_identifier: scope,
                    routing_action_identifier: action.identifier,
                    smtp_outbound_identifier: outbound.identifier,
                    smtp_outbound_name: outbound.name.clone(),
                    kind: action.kind,
                    status: if blocked.is_some() {
                        OutboundDeliveryStatus::Failed
                    } else {
                        OutboundDeliveryStatus::Pending
                    },
                    last_error: blocked,
                    recipients,
                    created_at: now,
                });
            }
        }
        deliveries
    }
}
