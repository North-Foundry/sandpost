//! Cancellable native notifications with session and message-visibility revalidation.

use crate::NativeRuntime;
use crate::boundary::{application_error, serialize};
use napi::Result;
use napi_derive::napi;
use serde_json::{Value, json};
use tokio::sync::broadcast;

#[napi]
impl NativeRuntime {
    /// Await an authorized notification without polling; unauthenticated calls wait for login.
    /// Committed state changes and lag require a mailbox resync. Cancellation or runtime
    /// termination returns null.
    #[napi]
    pub async fn next_event(&self) -> Result<Option<Value>> {
        let mut session_token = self.session_token.subscribe();
        let mut cancellation = self.cancel_events.subscribe();
        let mut stop = self.stop.subscribe();
        let mut completion = self.completion.clone();
        loop {
            if *cancellation.borrow() || *stop.borrow() || completion.borrow().is_some() {
                return Ok(None);
            }
            let token = match self.valid_session_token().await? {
                Some(token) => token,
                None => {
                    tokio::select! {
                        biased;
                        _ = cancellation.changed() => return Ok(None),
                        _ = stop.changed() => return Ok(None),
                        _ = completion.changed() => return Ok(None),
                        changed = session_token.changed() => if changed.is_err() { return Ok(None) },
                    }
                    continue;
                }
            };
            tokio::select! {
                biased;
                _ = cancellation.changed() => return Ok(None),
                _ = stop.changed() => return Ok(None),
                _ = completion.changed() => return Ok(None),
                changed = session_token.changed() => if changed.is_err() { return Ok(None) },
                event = async { self.events.lock().await.recv().await } => {
                    match event {
                        Ok(event) => {
                            let Some(user) =
                                self.application.instance().session(&token).await.ok()
                            else {
                                self.session_token.send_replace(None);
                                continue;
                            };
                            let visible = self.application.authorized(&user).is_message_visible(event.identifier)
                                .await.map_err(application_error)?;
                            if !visible { continue; }
                            tokio::select! {
                                biased;
                                _ = cancellation.changed() => return Ok(None),
                                _ = stop.changed() => return Ok(None),
                                _ = completion.changed() => return Ok(None),
                                changed = session_token.changed() => { if changed.is_err() { return Ok(None); } continue; },
                                result = self.application.wait_for_search() => result.map_err(application_error)?,
                            }
                            let visible = self.application.authorized(&user).is_message_visible(event.identifier)
                                .await.map_err(application_error)?;
                            if visible && self.session_is_current(&token).await? {
                                return serialize(json!({"type": "message", "id": event.identifier, "seq": event.sequence})).map(Some);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            if self.session_is_current(&token).await? {
                                return serialize(json!({"type": "resync"})).map(Some);
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => return Ok(None),
                    }
                }
                change = async { self.changes.lock().await.recv().await } => {
                    match change {
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {
                            if self.session_is_current(&token).await? {
                                return serialize(json!({"type": "resync"})).map(Some);
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => return Ok(None),
                    }
                }
            }
        }
    }

    /// Unsubscribe the native event consumer and wake any pending nextEvent promise.
    #[napi]
    pub fn cancel_events(&self) {
        self.cancel_events.send_replace(true);
    }
}
