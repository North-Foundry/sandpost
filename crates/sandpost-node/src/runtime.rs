//! Startup, listener supervision, shutdown, and the native runtime's lifetime.

use crate::NativeRuntime;
use crate::boundary::{instance_error, native_error};
use napi::{Error, Result, Status};
use napi_derive::napi;
use sandpost::{Configuration, SandpostRuntime, StartupSummary};
use std::{io::Write, path::PathBuf};
use tokio::sync::{Mutex, watch};

#[napi]
impl NativeRuntime {
    /// Start the shared HTTP/SMTP/IMAP runtime using environment settings and a platform data default.
    #[napi(factory)]
    pub async fn start(default_data_directory: String) -> Result<Self> {
        let mut configuration = Configuration::from_environment().map_err(native_error)?;
        if std::env::var_os("SANDPOST_DATA_DIR").is_none() {
            configuration.data_directory = PathBuf::from(default_data_directory);
            if std::env::var_os("SANDPOST_DATABASE_PATH").is_none() {
                configuration.database_path = configuration.data_directory.join("sandpost.sqlite3");
            }
        }
        let filter = tracing_subscriber::EnvFilter::try_new(&configuration.log_level)
            .map_err(native_error)?;
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_ansi(false)
            .try_init();
        let mut running = SandpostRuntime::start(configuration.clone())
            .await
            .map_err(native_error)?;
        let application = running.application.clone();
        let bootstrap_secret = application
            .instance()
            .bootstrap_secret_for_startup()
            .map_err(instance_error)?;
        let startup = StartupSummary {
            configuration: &configuration,
            mail_listen_address: running.mail_listen_address,
            mail_authentication_required: running.mail_authentication_required,
            mail_encryption: running.mail_encryption,
            web_listen_address: running.web_listen_address,
            imap: running.imap.as_ref(),
            bootstrap_secret: bootstrap_secret.as_deref(),
        };
        let mut stdout = std::io::stdout().lock();
        write!(stdout, "{startup}")
            .and_then(|()| stdout.flush())
            .map_err(native_error)?;
        let events = Mutex::new(application.subscribe());
        let changes = Mutex::new(application.subscribe_changes());
        let (session_token, _) = watch::channel(None);
        let smtp_address = running
            .mail_listen_address
            .map(|address| address.to_string());
        let http_address = running.web_listen_address.to_string();
        let (stop, mut stop_receiver) = watch::channel(false);
        let (cancel_events, _) = watch::channel(false);
        let (completed, completion) = watch::channel(None);
        tokio::spawn(async move {
            let result = tokio::select! {
                _ = stop_receiver.changed() => Ok(()),
                result = running.wait() => result,
            };
            let result = result
                .and(running.shutdown().await)
                .map_err(|error| error.to_string());
            let _ = completed.send(Some(result));
        });
        Ok(Self {
            application,
            session_token,
            events,
            changes,
            stop,
            cancel_events,
            completion,
            smtp_address,
            http_address,
        })
    }

    /// Report the global SMTP server's actual bind address at startup, including a
    /// system-allocated port, or null when the listener did not start.
    #[napi(getter)]
    pub fn smtp_address(&self) -> Option<String> {
        self.smtp_address.clone()
    }

    /// Report the shared HTTP listener available to external development tools.
    #[napi(getter)]
    pub fn http_address(&self) -> String {
        self.http_address.clone()
    }

    /// Observe listener termination and propagate runtime failures to the desktop host.
    #[napi]
    pub async fn wait(&self) -> Result<()> {
        let mut completion = self.completion.clone();
        loop {
            if let Some(result) = completion.borrow().clone() {
                return result.map_err(native_error);
            }
            completion.changed().await.map_err(native_error)?;
        }
    }

    /// Request shutdown once and await the shared bounded listener drain; safe to repeat.
    #[napi]
    pub async fn shutdown(&self) -> Result<()> {
        self.stop.send_replace(true);
        self.cancel_events();
        self.wait().await
    }
}

impl NativeRuntime {
    /// Reject operations after shutdown or unexpected runtime termination.
    pub(super) fn ensure_running(&self) -> Result<()> {
        if *self.stop.borrow() || self.completion.borrow().is_some() {
            return Err(Error::new(
                Status::GenericFailure,
                "SandPost runtime has stopped",
            ));
        }
        Ok(())
    }
}

impl Drop for NativeRuntime {
    /// A dropped native handle still requests bounded listener draining.
    fn drop(&mut self) {
        self.stop.send_replace(true);
        self.cancel_events.send_replace(true);
    }
}
