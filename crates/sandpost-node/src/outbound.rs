//! Adapters for SMTP Outbounds, scope routing actions and outbound delivery status. The
//! application allows them to Owners only and never returns SMTP Outbound passwords.

use crate::NativeRuntime;
use crate::boundary::{
    application_error, deserialize, parse_message_identifier, parse_outbound_delivery_identifier,
    parse_routing_action_identifier, parse_scope_identifier, parse_smtp_outbound_identifier,
    serialize,
};
use napi::{Error, Result, Status};
use napi_derive::napi;
use sandpost_application::{
    ApplicationError, OutboundDeliveryListRequest, RoutingActionRequest, SmtpOutboundRequest,
    SmtpOutboundTestRequest,
};
use serde_json::Value;

/**
 * Map outbound failures, naming the missing record instead of the generic message error.
 */
fn outbound_error(error: ApplicationError) -> Error {
    match error {
        ApplicationError::NotFound => Error::new(Status::GenericFailure, "item not found"),
        error => application_error(error),
    }
}

#[napi]
impl NativeRuntime {
    /**
     * Describe the instance's outbound delivery policy.
     */
    #[napi]
    pub async fn outbound_policy(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .outbound_policy()
                .await
                .map_err(outbound_error)?,
        )
    }

    /**
     * List SMTP Outbounds without passwords.
     */
    #[napi]
    pub async fn outbound_list(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .list_smtp_outbounds()
                .await
                .map_err(outbound_error)?,
        )
    }

    /**
     * Read one SMTP Outbound without its password.
     */
    #[napi]
    pub async fn outbound_get(&self, identifier: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .get_smtp_outbound(parse_smtp_outbound_identifier(&identifier)?)
                .await
                .map_err(outbound_error)?,
        )
    }

    /**
     * Create an SMTP Outbound; its password is encrypted and never returned.
     */
    #[napi]
    pub async fn outbound_create(&self, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: SmtpOutboundRequest = deserialize(input, "invalid SMTP Outbound")?;
        serialize(
            self.application
                .authorized(&user)
                .create_smtp_outbound(request)
                .await
                .map_err(outbound_error)?,
        )
    }

    /**
     * Replace an SMTP Outbound; an omitted password keeps the stored one.
     */
    #[napi]
    pub async fn outbound_update(&self, identifier: String, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: SmtpOutboundRequest = deserialize(input, "invalid SMTP Outbound")?;
        serialize(
            self.application
                .authorized(&user)
                .update_smtp_outbound(parse_smtp_outbound_identifier(&identifier)?, request)
                .await
                .map_err(outbound_error)?,
        )
    }

    /**
     * Enable or disable an SMTP Outbound.
     */
    #[napi]
    pub async fn outbound_set_enabled(&self, identifier: String, enabled: bool) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .set_smtp_outbound_enabled(parse_smtp_outbound_identifier(&identifier)?, enabled)
                .await
                .map_err(outbound_error)?,
        )
    }

    /**
     * Delete an SMTP Outbound that no routing action references.
     */
    #[napi]
    pub async fn outbound_delete(&self, identifier: String) -> Result<()> {
        let user = self.ensure_authenticated().await?;
        self.application
            .authorized(&user)
            .delete_smtp_outbound(parse_smtp_outbound_identifier(&identifier)?)
            .await
            .map_err(outbound_error)
    }

    /**
     * Test the connection of a saved SMTP Outbound without sending mail.
     */
    #[napi]
    pub async fn outbound_test(&self, identifier: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .test_smtp_outbound(parse_smtp_outbound_identifier(&identifier)?)
                .await
                .map_err(outbound_error)?,
        )
    }

    /**
     * Test unsaved SMTP Outbound settings, reusing a saved outbound's password when given.
     */
    #[napi]
    pub async fn outbound_test_draft(
        &self,
        input: Value,
        identifier: Option<String>,
    ) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let outbound: SmtpOutboundRequest = deserialize(input, "invalid SMTP Outbound")?;
        let identifier = identifier
            .as_deref()
            .map(parse_smtp_outbound_identifier)
            .transpose()?;
        serialize(
            self.application
                .authorized(&user)
                .test_smtp_outbound_draft(SmtpOutboundTestRequest {
                    outbound,
                    identifier,
                })
                .await
                .map_err(outbound_error)?,
        )
    }

    /**
     * List a scope's routing actions.
     */
    #[napi]
    pub async fn routing_list(&self, scope: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .list_routing_actions(parse_scope_identifier(&scope)?)
                .await
                .map_err(outbound_error)?,
        )
    }

    /**
     * Attach a relay or forward action to a scope.
     */
    #[napi]
    pub async fn routing_create(&self, scope: String, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: RoutingActionRequest = deserialize(input, "invalid routing action")?;
        serialize(
            self.application
                .authorized(&user)
                .create_routing_action(parse_scope_identifier(&scope)?, request)
                .await
                .map_err(outbound_error)?,
        )
    }

    /**
     * Replace a routing action.
     */
    #[napi]
    pub async fn routing_update(&self, identifier: String, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: RoutingActionRequest = deserialize(input, "invalid routing action")?;
        serialize(
            self.application
                .authorized(&user)
                .update_routing_action(parse_routing_action_identifier(&identifier)?, request)
                .await
                .map_err(outbound_error)?,
        )
    }

    /**
     * Delete a routing action.
     */
    #[napi]
    pub async fn routing_delete(&self, identifier: String) -> Result<()> {
        let user = self.ensure_authenticated().await?;
        self.application
            .authorized(&user)
            .delete_routing_action(parse_routing_action_identifier(&identifier)?)
            .await
            .map_err(outbound_error)
    }

    /**
     * List deliveries newest first, bounded by the caller's mail visibility.
     */
    #[napi]
    pub async fn deliveries_list(&self, options: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: OutboundDeliveryListRequest =
            deserialize(options, "invalid delivery listing")?;
        serialize(
            self.application
                .authorized(&user)
                .list_outbound_deliveries(request)
                .await
                .map_err(outbound_error)?,
        )
    }

    /**
     * Read one delivery with its attempt history.
     */
    #[napi]
    pub async fn delivery_get(&self, identifier: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .get_outbound_delivery(parse_outbound_delivery_identifier(&identifier)?)
                .await
                .map_err(outbound_error)?,
        )
    }

    /**
     * Schedule a failed delivery again.
     */
    #[napi]
    pub async fn delivery_retry(&self, identifier: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .retry_outbound_delivery(parse_outbound_delivery_identifier(&identifier)?)
                .await
                .map_err(outbound_error)?,
        )
    }

    /**
     * List the deliveries of one readable message.
     */
    #[napi]
    pub async fn message_deliveries(&self, message: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .list_message_deliveries(parse_message_identifier(&message)?)
                .await
                .map_err(outbound_error)?,
        )
    }
}
