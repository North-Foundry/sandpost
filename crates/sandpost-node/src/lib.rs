use napi_derive::napi;
use sandpost_application::{Application, ApplicationChange, MessageEvent};
use tokio::sync::{Mutex, broadcast, watch};

/// One Rust runtime owned by Electron main, with a cancellable application-event stream.
#[napi]
pub struct NativeRuntime {
    application: Application,
    session_token: watch::Sender<Option<String>>,
    events: Mutex<broadcast::Receiver<MessageEvent>>,
    changes: Mutex<broadcast::Receiver<ApplicationChange>>,
    stop: watch::Sender<bool>,
    cancel_events: watch::Sender<bool>,
    completion: watch::Receiver<Option<std::result::Result<(), String>>>,
    smtp_address: String,
    http_address: String,
}

// napi-rs must parse the exported class before expanding its factory in a child module.
mod authentication;
mod boundary;
mod events;
mod imap;
mod management;
mod messages;
mod personas;
mod runtime;

#[cfg(test)]
mod tests;
