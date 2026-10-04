//! Persist matched mail before publishing its notification.

use sandpost_core::Message;
use sandpost_match::Matcher;
use sandpost_storage::Storage;
use sandpost_web::MessageEvent;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Match and store one message on the blocking pool, then notify event subscribers.
///
/// Storage or worker failures prevent notification and propagate to the SMTP handler.
pub(crate) async fn capture_message(
    message: Message,
    matcher: Arc<Matcher>,
    storage: Storage,
    event_broadcast: broadcast::Sender<MessageEvent>,
) -> Result<(), String> {
    let identifier = message.identifier;
    // Matching and all SQLite operations stay off the async runtime workers.
    let sequence = tokio::task::spawn_blocking(move || {
        let result = matcher.match_message(&message.facts);
        tracing::debug!(
            candidates = result.statistics.candidates,
            evaluations = result.statistics.predicate_evaluations,
            matched = result.scopes.len(),
            "message matched"
        );
        storage.insert_message(&message, &result.scopes)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    let _ = event_broadcast.send(MessageEvent {
        identifier,
        sequence,
    });
    Ok(())
}
