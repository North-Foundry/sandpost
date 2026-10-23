//! SMTP Outbounds, scope routing actions and durable outbound delivery jobs.
use crate::{
    MessageIdentifier, NewOutboundDelivery, NewRoutingAction, NewSmtpOutbound,
    OutboundAttemptResult, OutboundDelivery, OutboundDeliveryAttempt, OutboundDeliveryIdentifier,
    OutboundDeliveryQuery, RoutingAction, RoutingActionIdentifier, RoutingActionUpdate,
    ScopeIdentifier, SmtpOutbound, SmtpOutboundCredential, SmtpOutboundIdentifier,
    SmtpOutboundPassword, SmtpOutboundSettings, StorageError,
};
use async_trait::async_trait;

/// Most terminal delivery jobs retained; finishing a job removes older terminal ones.
pub const OUTBOUND_DELIVERY_RETENTION: usize = 10_000;

/// Most attempts retained per delivery job; recording an attempt removes older ones.
pub const OUTBOUND_ATTEMPT_RETENTION: usize = 50;

/// External SMTP delivery configuration and the jobs it produces.
///
/// SMTP Outbounds are instance-level and reusable; routing actions belong to one scope (deleted
/// with it) and reference one SMTP Outbound, which cannot be deleted while referenced.
///
/// [`crate::MessageStorage::insert_message`] queues a routing evaluation for the new message in
/// the same transaction, but only while at least one enabled routing action uses an enabled SMTP
/// Outbound, so mail captured without routing configuration leaves no trace. The application
/// evaluates queued messages and turns them into delivery jobs with
/// [`OutboundStorage::complete_routing_evaluation`]. Physical message deletion cascades its queued
/// evaluation and its delivery jobs.
///
/// Reads are not user-scoped; callers enforce roles and message visibility. Encrypted passwords
/// are only returned by [`OutboundStorage::get_smtp_outbound_credential`].
#[async_trait]
pub trait OutboundStorage: Send + Sync {
    /**
     * List every SMTP Outbound ordered by name and identifier, without passwords.
     */
    async fn list_smtp_outbounds(&self) -> Result<Vec<SmtpOutbound>, StorageError>;

    /**
     * Load one SMTP Outbound without its password.
     */
    async fn get_smtp_outbound(
        &self,
        identifier: SmtpOutboundIdentifier,
    ) -> Result<Option<SmtpOutbound>, StorageError>;

    /**
     * Load one SMTP Outbound with its encrypted password, to connect to the server.
     */
    async fn get_smtp_outbound_credential(
        &self,
        identifier: SmtpOutboundIdentifier,
    ) -> Result<Option<SmtpOutboundCredential>, StorageError>;

    /**
     * Create an SMTP Outbound; a name already used, ignoring ASCII case, fails with
     * `DuplicateSmtpOutboundName`.
     */
    async fn create_smtp_outbound(
        &self,
        outbound: &NewSmtpOutbound,
    ) -> Result<SmtpOutbound, StorageError>;

    /**
     * Replace an SMTP Outbound's settings and apply the password change, returning `None` for
     * an unknown outbound. Disabling it fails its pending and retrying deliveries in the same
     * operation; attempts already in progress finish normally.
     */
    async fn update_smtp_outbound(
        &self,
        identifier: SmtpOutboundIdentifier,
        settings: &SmtpOutboundSettings,
        password: &SmtpOutboundPassword,
        updated_at: i64,
    ) -> Result<Option<SmtpOutbound>, StorageError>;

    /**
     * Delete an SMTP Outbound, returning whether it existed. One still referenced by a routing
     * action fails with `SmtpOutboundInUse`. Its pending and retrying deliveries fail and every
     * delivery keeps its recorded outbound name without the reference.
     */
    async fn delete_smtp_outbound(
        &self,
        identifier: SmtpOutboundIdentifier,
    ) -> Result<bool, StorageError>;

    /**
     * List routing actions grouped by scope in creation order, optionally only
     * those of one scope.
     */
    async fn list_routing_actions(
        &self,
        scope: Option<ScopeIdentifier>,
    ) -> Result<Vec<RoutingAction>, StorageError>;

    /**
     * Load one routing action.
     */
    async fn get_routing_action(
        &self,
        identifier: RoutingActionIdentifier,
    ) -> Result<Option<RoutingAction>, StorageError>;

    /**
     * Create a routing action; an unknown scope or SMTP Outbound fails with `NotFound`.
     */
    async fn create_routing_action(
        &self,
        action: &NewRoutingAction,
    ) -> Result<RoutingAction, StorageError>;

    /**
     * Replace a routing action's mutable fields, returning `None` for an unknown action; an
     * unknown SMTP Outbound fails with `NotFound`. Disabling the action or changing its type or
     * outbound fails its pending and retrying deliveries in the same operation.
     */
    async fn update_routing_action(
        &self,
        identifier: RoutingActionIdentifier,
        update: &RoutingActionUpdate,
        updated_at: i64,
    ) -> Result<Option<RoutingAction>, StorageError>;

    /**
     * Delete a routing action, returning whether it existed. Its pending and retrying
     * deliveries fail; recorded deliveries keep their history without the reference.
     */
    async fn delete_routing_action(
        &self,
        identifier: RoutingActionIdentifier,
    ) -> Result<bool, StorageError>;

    /**
     * List messages waiting for routing evaluation in queue order.
     */
    async fn list_routing_evaluations(
        &self,
        limit: usize,
    ) -> Result<Vec<MessageIdentifier>, StorageError>;

    /**
     * Atomically create the deliveries decided for a queued message and remove it from the
     * queue, returning how many deliveries were created. A delivery for a message and routing
     * action that already exists is not created again, so evaluation is idempotent. A message
     * that is no longer queued creates nothing.
     */
    async fn complete_routing_evaluation(
        &self,
        message: MessageIdentifier,
        deliveries: &[NewOutboundDelivery],
    ) -> Result<u64, StorageError>;

    /**
     * Mark every delivery left `processing` by a stopped process as `retrying`, due at
     * `interrupted_at`, and record its unfinished attempt as interrupted. Returns how many
     * changed. Call once before dispatching in a process that owns the installation.
     */
    async fn interrupt_outbound_deliveries(&self, interrupted_at: i64)
    -> Result<u64, StorageError>;

    /**
     * List `pending` and `retrying` deliveries due at `now`, earliest due first.
     */
    async fn list_due_outbound_deliveries(
        &self,
        now: i64,
        limit: usize,
    ) -> Result<Vec<OutboundDelivery>, StorageError>;

    /**
     * Start an attempt: a due `pending` or `retrying` delivery becomes `processing`, counts the
     * attempt and records it as started. Anything else returns `None`.
     */
    async fn claim_outbound_delivery(
        &self,
        identifier: OutboundDeliveryIdentifier,
        started_at: i64,
    ) -> Result<Option<OutboundDelivery>, StorageError>;

    /**
     * Record the end of the running attempt of a `processing` delivery and move it to the
     * resulting state, returning false when it was not processing. Older terminal deliveries
     * beyond [`OUTBOUND_DELIVERY_RETENTION`] and older attempts beyond
     * [`OUTBOUND_ATTEMPT_RETENTION`] are removed in the same operation.
     */
    async fn finish_outbound_delivery_attempt(
        &self,
        identifier: OutboundDeliveryIdentifier,
        result: &OutboundAttemptResult,
    ) -> Result<bool, StorageError>;

    /**
     * Schedule a `failed` delivery again: its failed recipients become pending, its attempt
     * budget restarts and it is due at `retried_at`. Returns `None` when the delivery is not
     * failed; an unknown delivery fails with `NotFound`.
     */
    async fn retry_outbound_delivery(
        &self,
        identifier: OutboundDeliveryIdentifier,
        retried_at: i64,
    ) -> Result<Option<OutboundDelivery>, StorageError>;

    /**
     * List deliveries newest first with their recipients, from one snapshot. With a reader,
     * only deliveries whose message satisfies the reader's filter and was not deleted by the
     * reader are listed, so delivery history never reveals unreadable mail.
     */
    async fn list_outbound_deliveries(
        &self,
        query: &OutboundDeliveryQuery,
    ) -> Result<Vec<OutboundDelivery>, StorageError>;

    /**
     * Load one delivery with its recipients and attempts in attempt order.
     */
    async fn get_outbound_delivery(
        &self,
        identifier: OutboundDeliveryIdentifier,
    ) -> Result<Option<(OutboundDelivery, Vec<OutboundDeliveryAttempt>)>, StorageError>;
}
