//! Personas: user-owned virtual recipients that run a PersonaScript on mail addressed to them.
//!
//! A Persona belongs to exactly one user and has an address unique across the installation.
//! Management follows Web folder ownership with an administrative exception: owners manage their
//! own Personas, Owners may also list, inspect, enable, disable, and delete any Persona, and
//! nobody else learns that another user's Persona exists. Viewers cannot create or change
//! Personas. A Persona never widens access: its view, history, and executions are bounded by the
//! reader's (or, for execution, the owner's) current composed authorization.
mod access;
mod runtime;

#[cfg(test)]
mod tests;

pub use access::AddressAccess;
pub(crate) use runtime::PersonaRuntime;
pub use runtime::PersonaSettings;

use crate::credentials::current_unix_seconds;
use crate::management::is_owner;
use crate::{
    Application, ApplicationError, AuthorizedApplication, MessageSummary, QueryOptions,
    authorization, validate_page_size, validated_name,
};
use sandpost_core::{
    Persona, PersonaExecution, PersonaExecutionAction, PersonaExecutionIdentifier,
    PersonaIdentifier, normalize_inbox_address,
};
use sandpost_query::{Expression, Field, Operator, Predicate, Value};
use sandpost_storage::{MessageListQuery, NewPersona, PersonaUpdate, StorageError};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::watch;

/// Most Personas one user may own.
pub const MAXIMUM_PERSONAS_PER_USER: usize = 50;

/// A Persona definition supplied by its owner.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaRequest {
    pub name: String,
    pub address: String,
    pub script: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

/// A request to enable or disable a Persona.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaEnabledRequest {
    pub enabled: bool,
}

/// PersonaScript source to validate without saving it.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaScriptRequest {
    pub script: String,
}

/// The result of validating PersonaScript source.
#[derive(Debug, Clone, Serialize)]
pub struct PersonaScriptValidation {
    pub valid: bool,
    pub error: Option<PersonaScriptDiagnostic>,
    /// Statements in a valid script, including nested ones.
    pub statements: Option<usize>,
    /// Longest sum of waits along any path of a valid script.
    pub longest_wait_milliseconds: Option<u64>,
}

/// The first problem in a script, with its position.
#[derive(Debug, Clone, Serialize)]
pub struct PersonaScriptDiagnostic {
    pub message: String,
    /// Zero-based UTF-8 byte offset.
    pub position: usize,
    pub line: u32,
    pub column: u32,
}

/// A Persona as returned to clients.
#[derive(Debug, Clone, Serialize)]
pub struct PersonaDetails {
    #[serde(flatten)]
    pub persona: Persona,
    /// Whether the owner's current access covers mail addressed to the Persona.
    pub address_access: AddressAccess,
    /// The newest execution whose message the reader can currently read.
    pub last_execution: Option<PersonaExecutionSummary>,
}

/// An execution with the subject of its message.
#[derive(Debug, Clone, Serialize)]
pub struct PersonaExecutionSummary {
    #[serde(flatten)]
    pub execution: PersonaExecution,
    pub subject: String,
}

/// An execution with every recorded step.
#[derive(Debug, Clone, Serialize)]
pub struct PersonaExecutionDetails {
    #[serde(flatten)]
    pub execution: PersonaExecution,
    pub subject: String,
    pub actions: Vec<PersonaExecutionAction>,
}

/**
 * Return the enabled state of a Persona created without an explicit choice.
 */
fn default_enabled() -> bool {
    true
}

impl Application {
    /**
     * Replace the limits and network policy used by Persona executions and script validation.
     */
    pub fn with_persona_settings(mut self, settings: PersonaSettings) -> Self {
        self.personas = Arc::new(PersonaRuntime::new(settings));
        self
    }

    /**
     * Run the Persona dispatcher until `shutdown` turns true.
     *
     * Hosts run this once per installation alongside their listeners. Executions interrupted by
     * shutdown stay recorded as running and resume when the dispatcher runs again.
     */
    pub async fn run_personas(&self, shutdown: watch::Receiver<bool>) {
        self.personas.clone().run(self.clone(), shutdown).await;
    }

    /**
     * Return the limits and network policy of Persona executions.
     */
    pub fn persona_settings(&self) -> &PersonaSettings {
        self.personas.settings()
    }
}

impl AuthorizedApplication {
    /**
     * List the caller's own Personas.
     */
    pub async fn list_personas(&self) -> Result<Vec<PersonaDetails>, ApplicationError> {
        self.account().await?;
        let personas = self
            .application
            .storage
            .list_personas(Some(self.user_identifier))
            .await?;
        self.describe_all(personas).await
    }

    /**
     * List every user's Personas; Owners only.
     */
    pub async fn list_all_personas(&self) -> Result<Vec<PersonaDetails>, ApplicationError> {
        self.require_owner().await?;
        let personas = self.application.storage.list_personas(None).await?;
        self.describe_all(personas).await
    }

    /**
     * Read one Persona owned by the caller, or any Persona for an Owner.
     */
    pub async fn get_persona(
        &self,
        identifier: PersonaIdentifier,
    ) -> Result<PersonaDetails, ApplicationError> {
        let persona = self.visible_persona(identifier).await?;
        self.describe(persona).await
    }

    /**
     * Create a Persona owned by the caller after validating its name, address, and script.
     *
     * The address must be valid and unique across the installation. When the caller's access
     * provably excludes every message to the address the Persona is refused; when access depends
     * on each message the Persona is created and authorization is enforced per execution.
     */
    pub async fn create_persona(
        &self,
        request: PersonaRequest,
    ) -> Result<PersonaDetails, ApplicationError> {
        self.require_mail_mutation().await?;
        let request = self.validate_request(request).await?;
        let storage = &self.application.storage;
        if storage
            .list_personas(Some(self.user_identifier))
            .await?
            .len()
            >= MAXIMUM_PERSONAS_PER_USER
        {
            return Err(ApplicationError::InvalidInput(format!(
                "a user can have at most {MAXIMUM_PERSONAS_PER_USER} personas"
            )));
        }
        let persona = storage
            .create_persona(&NewPersona {
                identifier: PersonaIdentifier::new(),
                owner_identifier: self.user_identifier,
                name: request.name,
                address: request.address,
                script: request.script,
                enabled: request.enabled,
                created_at: current_unix_seconds(),
            })
            .await
            .map_err(persona_storage_error)?;
        self.describe(persona).await
    }

    /**
     * Replace the caller's own Persona definition.
     *
     * Disabling the Persona or changing its address cancels its pending and running executions.
     * A script edit applies to executions that start afterwards; an execution already running
     * finishes with the script it started with.
     */
    pub async fn update_persona(
        &self,
        identifier: PersonaIdentifier,
        request: PersonaRequest,
    ) -> Result<PersonaDetails, ApplicationError> {
        self.require_mail_mutation().await?;
        let previous = self.visible_persona(identifier).await?;
        if previous.owner_identifier != self.user_identifier {
            return Err(ApplicationError::Forbidden);
        }
        let request = self.validate_request(request).await?;
        let updated = self
            .application
            .storage
            .update_persona(
                identifier,
                &PersonaUpdate {
                    name: request.name,
                    address: request.address,
                    script: request.script,
                    enabled: request.enabled,
                },
                current_unix_seconds(),
            )
            .await
            .map_err(persona_storage_error)?
            .ok_or(ApplicationError::NotFound)?;
        if !updated.enabled {
            self.cancel_executions(identifier, "the Persona was disabled")
                .await?;
        } else if updated.address != previous.address {
            self.cancel_executions(identifier, "the Persona's address changed")
                .await?;
        }
        self.describe(updated).await
    }

    /**
     * Enable or disable a Persona owned by the caller, or any Persona for an Owner.
     *
     * Disabling cancels pending and running executions; enabling never replays earlier mail.
     */
    pub async fn set_persona_enabled(
        &self,
        identifier: PersonaIdentifier,
        enabled: bool,
    ) -> Result<PersonaDetails, ApplicationError> {
        self.require_mail_mutation().await?;
        let persona = self.visible_persona(identifier).await?;
        if persona.enabled == enabled {
            return self.describe(persona).await;
        }
        let updated = self
            .application
            .storage
            .update_persona(
                identifier,
                &PersonaUpdate {
                    name: persona.name,
                    address: persona.address,
                    script: persona.script,
                    enabled,
                },
                current_unix_seconds(),
            )
            .await
            .map_err(persona_storage_error)?
            .ok_or(ApplicationError::NotFound)?;
        if !enabled {
            self.cancel_executions(identifier, "the Persona was disabled")
                .await?;
        }
        self.describe(updated).await
    }

    /**
     * Delete a Persona owned by the caller, or any Persona for an Owner, with its execution
     * history. Messages it received are kept.
     */
    pub async fn delete_persona(
        &self,
        identifier: PersonaIdentifier,
    ) -> Result<(), ApplicationError> {
        self.require_mail_mutation().await?;
        self.visible_persona(identifier).await?;
        self.application
            .personas
            .cancel_persona(identifier, "the Persona was deleted");
        if !self.application.storage.delete_persona(identifier).await? {
            return Err(ApplicationError::NotFound);
        }
        Ok(())
    }

    /**
     * Validate PersonaScript source under the runtime limits without saving or running it.
     */
    pub async fn validate_persona_script(
        &self,
        script: &str,
    ) -> Result<PersonaScriptValidation, ApplicationError> {
        self.account().await?;
        Ok(match self.application.personas.compile(script) {
            Ok(program) => PersonaScriptValidation {
                valid: true,
                error: None,
                statements: Some(program.statement_count()),
                longest_wait_milliseconds: Some(
                    u64::try_from(program.longest_total_wait().as_millis()).unwrap_or(u64::MAX),
                ),
            },
            Err(error) => PersonaScriptValidation {
                valid: false,
                error: Some(PersonaScriptDiagnostic {
                    message: error.message,
                    position: error.position,
                    line: error.line,
                    column: error.column,
                }),
                statements: None,
                longest_wait_milliseconds: None,
            },
        })
    }

    /**
     * List a visible Persona's executions newest first, limited to mail the caller can read.
     */
    pub async fn list_persona_executions(
        &self,
        identifier: PersonaIdentifier,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<PersonaExecutionSummary>, ApplicationError> {
        validate_page_size(limit)?;
        self.visible_persona(identifier).await?;
        let filter = self.reader_filter().await?;
        Ok(self
            .application
            .storage
            .list_persona_executions(identifier, self.user_identifier, &filter, before, limit)
            .await?
            .into_iter()
            .map(|entry| PersonaExecutionSummary {
                execution: entry.execution,
                subject: entry.subject,
            })
            .collect())
    }

    /**
     * Read one execution with its steps when the caller can see the Persona and read the message.
     */
    pub async fn get_persona_execution(
        &self,
        persona: PersonaIdentifier,
        execution: PersonaExecutionIdentifier,
    ) -> Result<PersonaExecutionDetails, ApplicationError> {
        self.visible_persona(persona).await?;
        let storage = &self.application.storage;
        let (record, actions) = storage
            .get_persona_execution(execution)
            .await?
            .filter(|(record, _)| record.persona_identifier == persona)
            .ok_or(ApplicationError::NotFound)?;
        let (facts, _) = storage
            .get_user_message_metadata(self.user_identifier, record.message_identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        authorization::require_snapshot(storage, self.user_identifier, &facts).await?;
        Ok(PersonaExecutionDetails {
            execution: record,
            subject: facts.subject,
            actions,
        })
    }

    /**
     * List the caller's readable mail addressed to a visible Persona, newest first.
     *
     * Like a Smart Folder, the view includes filed mail and excludes Trash and deleted mail; an
     * optional query further restricts it.
     */
    pub async fn query_persona_messages(
        &self,
        identifier: PersonaIdentifier,
        options: QueryOptions,
    ) -> Result<Vec<MessageSummary>, ApplicationError> {
        validate_page_size(options.limit)?;
        let persona = self.visible_persona(identifier).await?;
        let query = authorization::compose(
            &self.application.storage,
            self.user_identifier,
            &QueryOptions {
                view_identifier: None,
                ..options.clone()
            },
            None,
        )
        .await?;
        let filter = Expression::And(vec![
            authorization::storage_filter(&query),
            persona_recipient(&persona.address),
        ]);
        Ok(self
            .application
            .storage
            .list_user_messages(
                self.user_identifier,
                MessageListQuery {
                    filter: Some(filter),
                    before: options.before,
                    limit: options.limit,
                },
            )
            .await?)
    }

    /**
     * Load a Persona the caller owns, or any Persona for an Owner; others are not found.
     */
    async fn visible_persona(
        &self,
        identifier: PersonaIdentifier,
    ) -> Result<Persona, ApplicationError> {
        let account = self.account().await?;
        self.application
            .storage
            .get_persona(identifier)
            .await?
            .filter(|persona| {
                persona.owner_identifier == self.user_identifier || is_owner(account.global_role)
            })
            .ok_or(ApplicationError::NotFound)
    }

    /**
     * Normalize and validate a Persona definition for the caller.
     */
    async fn validate_request(
        &self,
        request: PersonaRequest,
    ) -> Result<PersonaRequest, ApplicationError> {
        let name = validated_name(&request.name)?;
        let address = normalize_inbox_address(&request.address).ok_or_else(|| {
            ApplicationError::InvalidInput(
                "address must be a valid ASCII email address with a 1 to 64 character dot-atom local part"
                    .into(),
            )
        })?;
        if let Err(error) = self.application.personas.compile(&request.script) {
            return Err(ApplicationError::InvalidInput(format!(
                "invalid PersonaScript at {error}"
            )));
        }
        let access =
            access::address_access(&self.application.storage, self.user_identifier, &address)
                .await?;
        if access == AddressAccess::Denied {
            return Err(ApplicationError::InvalidInput(
                "this address is outside your mail access, so the Persona could never act".into(),
            ));
        }
        Ok(PersonaRequest {
            name,
            address,
            script: request.script,
            enabled: request.enabled,
        })
    }

    /**
     * Cancel a Persona's stored and in-process executions.
     */
    async fn cancel_executions(
        &self,
        identifier: PersonaIdentifier,
        reason: &str,
    ) -> Result<(), ApplicationError> {
        self.application.personas.cancel_persona(identifier, reason);
        self.application
            .storage
            .cancel_persona_executions(identifier, reason, current_unix_seconds())
            .await?;
        Ok(())
    }

    /**
     * Return the caller's composed authorization for filtering history.
     */
    async fn reader_filter(&self) -> Result<Expression, ApplicationError> {
        let query = authorization::compose(
            &self.application.storage,
            self.user_identifier,
            &QueryOptions::default(),
            None,
        )
        .await?;
        Ok(authorization::storage_filter(&query))
    }

    /**
     * Describe several Personas with one authorization composition for the caller.
     */
    async fn describe_all(
        &self,
        personas: Vec<Persona>,
    ) -> Result<Vec<PersonaDetails>, ApplicationError> {
        let filter = self.reader_filter().await?;
        let mut described = Vec::with_capacity(personas.len());
        for persona in personas {
            described.push(self.describe_with(persona, &filter).await?);
        }
        Ok(described)
    }

    /**
     * Describe one Persona for the caller.
     */
    async fn describe(&self, persona: Persona) -> Result<PersonaDetails, ApplicationError> {
        let filter = self.reader_filter().await?;
        self.describe_with(persona, &filter).await
    }

    /**
     * Add the owner's address access and the reader's newest visible execution.
     */
    async fn describe_with(
        &self,
        persona: Persona,
        filter: &Expression,
    ) -> Result<PersonaDetails, ApplicationError> {
        let storage = &self.application.storage;
        let address_access = match storage.get_user(persona.owner_identifier).await? {
            Some(owner) => {
                access::address_access(storage, owner.identifier, &persona.address).await?
            }
            None => AddressAccess::Denied,
        };
        let last_execution = storage
            .list_persona_executions(persona.identifier, self.user_identifier, filter, None, 1)
            .await?
            .into_iter()
            .next()
            .map(|entry| PersonaExecutionSummary {
                execution: entry.execution,
                subject: entry.subject,
            });
        Ok(PersonaDetails {
            persona,
            address_access,
            last_execution,
        })
    }
}

/**
 * Build the canonical predicate selecting mail whose envelope includes a Persona's address.
 */
fn persona_recipient(address: &str) -> Expression {
    Expression::Predicate(Predicate {
        field: Field::EnvelopeToAddress,
        operator: Operator::Equal,
        value: Value::String(address.to_ascii_lowercase()),
    })
}

/**
 * Map Persona storage failures to stable application errors.
 *
 * A duplicate address reveals only that the address is taken, which global uniqueness cannot
 * avoid; it never names the Persona or its owner.
 */
fn persona_storage_error(error: StorageError) -> ApplicationError {
    match error {
        StorageError::DuplicatePersonaAddress => ApplicationError::InUse(
            "this email address is already used by a persona; choose another address".into(),
        ),
        StorageError::NotFound => ApplicationError::NotFound,
        error => error.into(),
    }
}
