//! What a Persona's owner may reach: the address-level preview shown when editing a Persona, and
//! the per-message eligibility checked before and during every execution.
//!
//! Scopes are arbitrary queries, so an address alone rarely decides access. The preview evaluates
//! the owner's composed authorization with three-valued logic over every message that could be
//! addressed to the Persona: only envelope-recipient predicates can be decided from the address,
//! because every such message lists it among its envelope recipients. Everything else is
//! unknown until a real message arrives. Execution never relies on the preview: it authorizes the
//! actual message against the owner's current authorization, exactly like a direct read.
use crate::{ApplicationError, QueryOptions, authorization};
use sandpost_core::{
    GlobalRole, Mailbox, MessageFacts, MessageIdentifier, Persona, PersonaIdentifier,
    UserIdentifier,
};
use sandpost_query::{Expression, Field};
use sandpost_storage::Storage;
use serde::Serialize;
use std::sync::Arc;

/// Whether the owner's current access covers mail addressed to a Persona.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressAccess {
    /// Every message addressed to the Persona is readable by the owner.
    Confirmed,
    /// Access depends on other facts of each message, so it is decided per message.
    PerMessage,
    /// No message addressed to the Persona can be readable by the owner.
    Denied,
}

/// Kleene truth value of an expression over every message addressed to one recipient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Truth {
    True,
    False,
    Unknown,
}

/**
 * Decide how the owner's composed authorization applies to mail addressed to `address`.
 */
pub(crate) async fn address_access(
    storage: &Arc<dyn Storage>,
    owner: UserIdentifier,
    address: &str,
) -> Result<AddressAccess, ApplicationError> {
    let query = authorization::compose(storage, owner, &QueryOptions::default(), None).await?;
    Ok(classify_address(
        &authorization::storage_filter(&query),
        address,
    ))
}

/**
 * Classify an authorization expression for messages whose envelope includes `address`.
 */
pub(crate) fn classify_address(expression: &Expression, address: &str) -> AddressAccess {
    let recipient = MessageFacts {
        envelope_to: vec![Mailbox {
            address: address.to_owned(),
            domain: address
                .rsplit_once('@')
                .map(|(_, domain)| domain.to_owned())
                .unwrap_or_default(),
        }],
        ..MessageFacts::default()
    };
    match truth(expression, &recipient) {
        Truth::True => AddressAccess::Confirmed,
        Truth::False => AddressAccess::Denied,
        Truth::Unknown => AddressAccess::PerMessage,
    }
}

/**
 * Evaluate an expression with three-valued logic.
 *
 * An envelope-recipient predicate satisfied by the Persona's own recipient is certainly true,
 * because comparisons on collections succeed when any value matches. An unsatisfied one is
 * unknown: another recipient of the same message might satisfy it. Predicates on any other
 * field are unknown. Single predicates are evaluated with `sandpost-query` itself, so the
 * preview cannot drift from the real query semantics.
 */
fn truth(expression: &Expression, recipient: &MessageFacts) -> Truth {
    match expression {
        Expression::True => Truth::True,
        Expression::False => Truth::False,
        Expression::Not(operand) => match truth(operand, recipient) {
            Truth::True => Truth::False,
            Truth::False => Truth::True,
            Truth::Unknown => Truth::Unknown,
        },
        Expression::And(operands) => {
            let mut result = Truth::True;
            for operand in operands {
                match truth(operand, recipient) {
                    Truth::False => return Truth::False,
                    Truth::Unknown => result = Truth::Unknown,
                    Truth::True => {}
                }
            }
            result
        }
        Expression::Or(operands) => {
            let mut result = Truth::False;
            for operand in operands {
                match truth(operand, recipient) {
                    Truth::True => return Truth::True,
                    Truth::Unknown => result = Truth::Unknown,
                    Truth::False => {}
                }
            }
            result
        }
        Expression::Predicate(predicate) => match predicate.field {
            Field::EnvelopeToAddress | Field::EnvelopeToDomain => {
                if expression.evaluate(recipient) {
                    Truth::True
                } else {
                    Truth::Unknown
                }
            }
            _ => Truth::Unknown,
        },
    }
}

/// Why a Persona may not act on a message right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Denial {
    PersonaMissing,
    OwnerMissing,
    MessageInaccessible,
    OwnerReadOnly,
    PersonaDisabled,
    AddressMismatch,
}

impl Denial {
    /**
     * Explain the denial for execution history.
     */
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::PersonaMissing => "the Persona was deleted",
            Self::OwnerMissing => "the Persona's owner no longer exists",
            Self::MessageInaccessible => "the Persona's owner can no longer access this message",
            Self::OwnerReadOnly => "the Persona's owner is a read-only Viewer",
            Self::PersonaDisabled => "the Persona was disabled",
            Self::AddressMismatch => {
                "the message is not addressed to the Persona's current address"
            }
        }
    }
}

/// A Persona allowed to act on a message, with the facts authorized for it.
pub(crate) struct Allowed {
    pub(crate) persona: Persona,
    pub(crate) facts: MessageFacts,
}

/**
 * Decide from current authoritative state whether a Persona may act on a message.
 *
 * The owner must exist and be able to read the message now (mail access, scopes with inherited
 * filters, personal filter, and the owner's own deletions), must not be a read-only Viewer, the
 * Persona must be enabled, and the message's envelope must still include the Persona's address.
 * Message access is checked before anything that would reveal Persona state, so callers can
 * discard executions for unreadable mail without exposing them.
 */
pub(crate) async fn eligibility(
    storage: &Arc<dyn Storage>,
    persona: PersonaIdentifier,
    message: MessageIdentifier,
) -> Result<Result<Allowed, Denial>, ApplicationError> {
    let Some(persona) = storage.get_persona(persona).await? else {
        return Ok(Err(Denial::PersonaMissing));
    };
    let Some(owner) = storage.get_user(persona.owner_identifier).await? else {
        return Ok(Err(Denial::OwnerMissing));
    };
    let Some((facts, _)) = storage
        .get_user_message_metadata(owner.identifier, message)
        .await?
    else {
        return Ok(Err(Denial::MessageInaccessible));
    };
    let query =
        authorization::compose(storage, owner.identifier, &QueryOptions::default(), None).await?;
    if !authorization::matches_snapshot(&query, &facts) {
        return Ok(Err(Denial::MessageInaccessible));
    }
    if owner.global_role == GlobalRole::Viewer {
        return Ok(Err(Denial::OwnerReadOnly));
    }
    if !persona.enabled {
        return Ok(Err(Denial::PersonaDisabled));
    }
    if !facts
        .envelope_to
        .iter()
        .any(|recipient| recipient.address.eq_ignore_ascii_case(&persona.address))
    {
        return Ok(Err(Denial::AddressMismatch));
    }
    Ok(Ok(Allowed { persona, facts }))
}
