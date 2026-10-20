//! Adapters for the signed-in user's Personas, PersonaScript validation, the Persona mail view,
//! and execution history. The application enforces ownership and mail visibility.

use crate::NativeRuntime;
use crate::boundary::{
    application_error, deserialize, page_size_from_number, parse_persona_execution_identifier,
    parse_persona_identifier, sequence_from_number, serialize,
};
use napi::{Error, Result, Status};
use napi_derive::napi;
use sandpost_application::{ApplicationError, PersonaRequest, QueryOptions};
use serde_json::Value;

/**
 * Map Persona failures, naming the missing record instead of the generic message error.
 */
fn persona_error(error: ApplicationError) -> Error {
    match error {
        ApplicationError::NotFound => Error::new(Status::GenericFailure, "persona not found"),
        error => application_error(error),
    }
}

/**
 * Convert an optional JavaScript execution cursor into the stored sequence it names.
 */
fn execution_cursor(before: Option<f64>) -> Result<Option<u64>> {
    before
        .map(sequence_from_number)
        .transpose()
        .map(|sequence| sequence.map(|sequence| sequence.0))
}

#[napi]
impl NativeRuntime {
    /**
     * List the signed-in user's Personas.
     */
    #[napi]
    pub async fn personas_list(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .list_personas()
                .await
                .map_err(persona_error)?,
        )
    }

    /**
     * Read one Persona the user may see.
     */
    #[napi]
    pub async fn persona_get(&self, identifier: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .get_persona(parse_persona_identifier(&identifier)?)
                .await
                .map_err(persona_error)?,
        )
    }

    /**
     * Create a Persona owned by the signed-in user.
     */
    #[napi]
    pub async fn persona_create(&self, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: PersonaRequest = deserialize(input, "invalid persona")?;
        serialize(
            self.application
                .authorized(&user)
                .create_persona(request)
                .await
                .map_err(persona_error)?,
        )
    }

    /**
     * Replace one of the signed-in user's Personas.
     */
    #[napi]
    pub async fn persona_update(&self, identifier: String, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: PersonaRequest = deserialize(input, "invalid persona")?;
        serialize(
            self.application
                .authorized(&user)
                .update_persona(parse_persona_identifier(&identifier)?, request)
                .await
                .map_err(persona_error)?,
        )
    }

    /**
     * Enable or disable a Persona.
     */
    #[napi]
    pub async fn persona_set_enabled(&self, identifier: String, enabled: bool) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .set_persona_enabled(parse_persona_identifier(&identifier)?, enabled)
                .await
                .map_err(persona_error)?,
        )
    }

    /**
     * Delete a Persona and its history without deleting its mail.
     */
    #[napi]
    pub async fn persona_delete(&self, identifier: String) -> Result<()> {
        let user = self.ensure_authenticated().await?;
        self.application
            .authorized(&user)
            .delete_persona(parse_persona_identifier(&identifier)?)
            .await
            .map_err(persona_error)
    }

    /**
     * Validate PersonaScript without saving it.
     */
    #[napi]
    pub async fn persona_validate(&self, script: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .validate_persona_script(&script)
                .await
                .map_err(persona_error)?,
        )
    }

    /**
     * List readable mail addressed to a Persona with the native keyset and query bounds.
     */
    #[napi]
    pub async fn persona_messages(
        &self,
        identifier: String,
        before: Option<f64>,
        limit: Option<f64>,
        query: Option<String>,
    ) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .query_persona_messages(
                    parse_persona_identifier(&identifier)?,
                    QueryOptions {
                        view_identifier: None,
                        query,
                        before: before.map(sequence_from_number).transpose()?,
                        limit: page_size_from_number(limit.unwrap_or(50.0))?,
                    },
                )
                .await
                .map_err(persona_error)?,
        )
    }

    /**
     * List a Persona's executions whose messages the user can read, newest first.
     */
    #[napi]
    pub async fn persona_executions(
        &self,
        identifier: String,
        before: Option<f64>,
        limit: Option<f64>,
    ) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .list_persona_executions(
                    parse_persona_identifier(&identifier)?,
                    execution_cursor(before)?,
                    page_size_from_number(limit.unwrap_or(50.0))?,
                )
                .await
                .map_err(persona_error)?,
        )
    }

    /**
     * Read one execution with its recorded steps.
     */
    #[napi]
    pub async fn persona_execution(&self, identifier: String, execution: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .get_persona_execution(
                    parse_persona_identifier(&identifier)?,
                    parse_persona_execution_identifier(&execution)?,
                )
                .await
                .map_err(persona_error)?,
        )
    }
}
