//! Handing a received message to the application and choosing the reply to `DATA`.
use super::{SessionConfiguration, trace::TraceHeaders};
use crate::{DeliveryError, SessionHandler};
use sandpost_mime::{Envelope, RawMessage};
use std::sync::Arc;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::timeout;

/// Parse a received message, give it to the handler, and return the DATA reply.
///
/// Parsing runs on the blocking pool. The reply is `250` only after the handler confirms
/// persistence; parse failures are permanent (`550`), refusals are `554`, and storage failures or
/// a handler that exceeds its time limit are temporary (`451`).
pub(crate) async fn deliver<Handler: SessionHandler>(
    configuration: Arc<SessionConfiguration<Handler>>,
    principal: Option<Handler::Principal>,
    envelope: Envelope,
    raw_message: Vec<u8>,
    trace_headers: TraceHeaders,
    connection_permit: Arc<OwnedSemaphorePermit>,
) -> String {
    let limits = configuration.limits;
    let maximum_message_size = limits.maximum_message_size;
    let parsed = tokio::task::spawn_blocking(move || {
        // Keep this connection admitted until the non-cancellable work has actually finished.
        let _connection_permit = connection_permit;
        RawMessage::new(raw_message)
            .envelope(envelope)
            .size_limit(maximum_message_size)
            .parse()
            .map(|mut message| {
                // Parse the submitted bytes first: adding trace fields must not turn malformed input
                // into a valid message or count server metadata against the advertised SIZE limit.
                trace_headers.prepend(&mut message);
                message
            })
    })
    .await;
    let message = match parsed {
        Ok(Ok(message)) => message,
        Ok(Err(error)) => return format!("550 5.6.0 {}", single_line(&error.to_string())),
        Err(error) => {
            tracing::error!(%error, "SMTP MIME parser task failed");
            return "451 4.3.0 message parsing failed".into();
        }
    };
    match timeout(
        limits.handler_timeout,
        configuration.handler.deliver(message, principal),
    )
    .await
    {
        Ok(Ok(())) => "250 2.0.0 message accepted".into(),
        Ok(Err(DeliveryError::Rejected(reason))) => format!("554 5.7.1 {}", single_line(&reason)),
        Ok(Err(error @ DeliveryError::Temporary(_))) => {
            tracing::warn!(%error, "SMTP message persistence failed");
            "451 4.3.0 persistence failed".into()
        }
        Err(_) => "451 4.3.0 persistence timed out".into(),
    }
}

/// Keep an application- or parser-supplied reason on one protocol line of bounded length.
fn single_line(reason: &str) -> String {
    reason
        .chars()
        .map(|character| {
            if !character.is_ascii() || character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(200)
        .collect()
}
