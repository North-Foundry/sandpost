//! Authorized server-sent events with session revalidation and resynchronization.

use crate::{ApplicationState, authentication::SessionToken};
use axum::{
    extract::{Extension, State},
    response::{
        Sse,
        sse::{Event, KeepAlive},
    },
};
use sandpost_application::SessionUser;
use std::{convert::Infallible, time::Duration};
use tokio_stream::StreamExt;

/// Stream message notifications and resynchronization events to subscribers.
pub(super) async fn events(
    State(state): State<ApplicationState>,
    Extension(user): Extension<SessionUser>,
    Extension(SessionToken(token)): Extension<SessionToken>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let instance = state.instance().clone();
    let event_state = state.clone();
    let session_user_identifier = user.id;
    let updates = tokio_stream::wrappers::BroadcastStream::new(state.subscribe())
        .then(move |update| {
            let instance = instance.clone();
            let state = event_state.clone();
            let token = token.clone();
            let session_user_identifier = session_user_identifier;
            async move {
                let Ok(user) = instance.session(&token).await else {
                    return (None, true);
                };
                if user.id != session_user_identifier {
                    return (None, true);
                }
                let event = match update {
                    Ok(message) => {
                        let authorized = state.authorized(&user);
                        if !matches!(
                            authorized.is_message_visible(message.identifier).await,
                            Ok(true)
                        ) {
                            return (None, false);
                        }
                        if let Err(error) = state.wait_for_search().await {
                            tracing::error!(%error, "event search synchronization failed");
                            return (None, true);
                        }
                        match authorized.is_message_visible(message.identifier).await {
                            Ok(true) if instance.session(&token).await.is_ok() => Event::default()
                                .event("message")
                                .id(message.sequence.0.to_string())
                                .json_data(message)
                                .map_err(|error| {
                                    tracing::error!(%error, "event serialization failed");
                                    Event::default().event("resync").data("{}")
                                })
                                .unwrap_or_else(|event| event),
                            Ok(_) => return (None, false),
                            Err(error) => {
                                tracing::error!(%error, "event visibility check failed");
                                return (
                                    Some(Ok(Event::default().event("resync").data("{}"))),
                                    false,
                                );
                            }
                        }
                    }
                    Err(_) => Event::default().event("resync").data("{}"),
                };
                (Some(Ok(event)), false)
            }
        })
        .take_while(|(_, stop)| !*stop)
        .filter_map(|(event, _)| event);
    let ready = tokio_stream::once(state).then(|state| async move {
        let event = match state.wait_for_search().await {
            Ok(()) => "ready",
            Err(_) => "resync",
        };
        Ok(Event::default().event(event).data("{}"))
    });
    Sse::new(ready.chain(updates)).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
