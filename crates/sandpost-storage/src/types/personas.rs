//! Persona storage types.
use sandpost_core::{PersonaExecution, PersonaIdentifier, UserIdentifier};

/// A new Persona with a normalized address and already validated script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPersona {
    pub identifier: PersonaIdentifier,
    pub owner_identifier: UserIdentifier,
    pub name: String,
    pub address: String,
    pub script: String,
    pub enabled: bool,
    pub created_at: i64,
}

/// A complete replacement of a Persona's mutable fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaUpdate {
    pub name: String,
    pub address: String,
    pub script: String,
    pub enabled: bool,
}

/// An execution in a history listing with the subject of its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaExecutionEntry {
    pub execution: PersonaExecution,
    pub subject: String,
}
