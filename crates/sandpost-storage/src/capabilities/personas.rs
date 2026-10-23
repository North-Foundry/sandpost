//! Persona definitions and their durable execution history.
use crate::{
    FilterExpression, NewPersona, Persona, PersonaExecution, PersonaExecutionAction,
    PersonaExecutionEntry, PersonaExecutionIdentifier, PersonaExecutionStatus, PersonaIdentifier,
    PersonaUpdate, StorageError, UserIdentifier,
};
use async_trait::async_trait;

/// Most terminal executions retained per Persona; finishing an execution removes older ones.
pub const PERSONA_EXECUTION_RETENTION: usize = 500;

/// Most pending executions per Persona; a message arriving beyond it schedules nothing for that
/// Persona, so a mail burst cannot queue unbounded outbound work.
pub const PERSONA_PENDING_EXECUTION_LIMIT: usize = 1000;

/// User-owned Personas and the executions scheduled when mail addressed to them is stored.
///
/// Executions are scheduled atomically by [`crate::MessageStorage::insert_message`]: one
/// `pending` execution per enabled Persona whose address equals, ignoring ASCII case, one of the
/// message's SMTP envelope recipients, at most one per Persona and message, and only while that
/// Persona has fewer than [`PERSONA_PENDING_EXECUTION_LIMIT`] pending executions. Scheduling does
/// not authorize anything: callers verify the owner's access before acting on the message.
///
/// Reads are not user-scoped; callers enforce ownership and message visibility. Deleting a
/// Persona deletes its executions; deleting a message deletes its executions; deleting the owner
/// deletes the Persona. Messages are never deleted because of a Persona.
#[async_trait]
pub trait PersonaStorage: Send + Sync {
    /**
     * Create a Persona. A normalized address already used by any Persona fails with
     * `DuplicatePersonaAddress`, enforced by the backend even under concurrent creation; an
     * unknown owner fails with `NotFound`.
     */
    async fn create_persona(&self, persona: &NewPersona) -> Result<Persona, StorageError>;

    /**
     * Load one Persona.
     */
    async fn get_persona(
        &self,
        identifier: PersonaIdentifier,
    ) -> Result<Option<Persona>, StorageError>;

    /**
     * List one owner's Personas, or every Persona when `owner` is `None`, ordered by name and
     * then identifier.
     */
    async fn list_personas(
        &self,
        owner: Option<UserIdentifier>,
    ) -> Result<Vec<Persona>, StorageError>;

    /**
     * Replace a Persona's mutable fields and advance its revision except for icon-only edits.
     * Returns `None` for an unknown Persona; an address used by another Persona fails with
     * `DuplicatePersonaAddress`.
     */
    async fn update_persona(
        &self,
        identifier: PersonaIdentifier,
        update: &PersonaUpdate,
        updated_at: i64,
    ) -> Result<Option<Persona>, StorageError>;

    /**
     * Delete a Persona and its executions, returning whether it existed.
     */
    async fn delete_persona(&self, identifier: PersonaIdentifier) -> Result<bool, StorageError>;

    /**
     * List executions in one status in scheduling order (ascending sequence), after an optional
     * exclusive sequence cursor.
     */
    async fn list_persona_executions_in_status(
        &self,
        status: PersonaExecutionStatus,
        after: Option<u64>,
        limit: usize,
    ) -> Result<Vec<PersonaExecution>, StorageError>;

    /**
     * Start an execution: a pending execution becomes running with `started_at` and the Persona
     * revision it will run. An execution that is already running is returned unchanged so an
     * interrupted run can resume. Terminal and unknown executions return `None`.
     */
    async fn start_persona_execution(
        &self,
        identifier: PersonaExecutionIdentifier,
        started_at: i64,
        persona_revision: u64,
    ) -> Result<Option<PersonaExecution>, StorageError>;

    /**
     * Insert or replace one step of an execution, keyed by its step number. An unknown execution
     * fails with `NotFound`.
     */
    async fn record_persona_execution_action(
        &self,
        identifier: PersonaExecutionIdentifier,
        action: &PersonaExecutionAction,
    ) -> Result<(), StorageError>;

    /**
     * Move a pending or running execution to a terminal status, returning false when it was
     * already terminal or does not exist. Steps still marked started become cancelled for a
     * cancelled execution and failed otherwise. Older terminal executions of the same Persona
     * beyond [`PERSONA_EXECUTION_RETENTION`] are removed in the same operation.
     */
    async fn finish_persona_execution(
        &self,
        identifier: PersonaExecutionIdentifier,
        status: PersonaExecutionStatus,
        completed_at: i64,
        error_message: Option<&str>,
    ) -> Result<bool, StorageError>;

    /**
     * Cancel every pending and running execution of a Persona with a reason, returning how many
     * changed. Their steps still marked started become cancelled.
     */
    async fn cancel_persona_executions(
        &self,
        persona: PersonaIdentifier,
        reason: &str,
        completed_at: i64,
    ) -> Result<u64, StorageError>;

    /**
     * Permanently remove one execution and its steps, returning whether it existed. Used for
     * executions whose message the owner could not read when they were first considered, so they
     * never become visible history.
     */
    async fn discard_persona_execution(
        &self,
        identifier: PersonaExecutionIdentifier,
    ) -> Result<bool, StorageError>;

    /**
     * List a Persona's executions newest first with an exclusive sequence cursor, keeping only
     * those whose message satisfies `filter` and has not been deleted by `viewer`, each with its
     * message subject read in the same snapshot. The filter must include the viewer's composed
     * authorization so history never reveals unreadable mail.
     */
    async fn list_persona_executions(
        &self,
        persona: PersonaIdentifier,
        viewer: UserIdentifier,
        filter: &FilterExpression,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<PersonaExecutionEntry>, StorageError>;

    /**
     * Load one execution and its steps in step order from one snapshot.
     */
    async fn get_persona_execution(
        &self,
        identifier: PersonaExecutionIdentifier,
    ) -> Result<Option<(PersonaExecution, Vec<PersonaExecutionAction>)>, StorageError>;
}
