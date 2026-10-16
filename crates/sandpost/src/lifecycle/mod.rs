//! Bind, supervise, and drain shared HTTP, SMTP, and IMAP transports without owning signals.

use crate::smtp_delivery::MailDelivery;
use crate::smtp_listener::{MailEncryption, MailServer, authentication_policy};
use crate::{Configuration, bootstrap};
use sandpost_application::{Application, Instance, SearchCoordinator, SmtpAuthenticator};
use sandpost_smtp::AuthenticationPolicy;
use std::{error::Error, net::SocketAddr, sync::Arc};
use tokio::{
    net::TcpListener,
    sync::{broadcast, watch},
    task::JoinHandle,
};

mod http;
mod indexing;
mod supervision;

use indexing::indexing_worker;
use supervision::{run_shutdown_cleanup, supervise, wait_for_shutdown};

/// Startup, transport, or worker failure reported to the embedding host.
pub type RuntimeError = Box<dyn Error + Send + Sync>;

/// Entry point for starting the same catcher in CLI and desktop hosts.
pub struct SandpostRuntime;

/// Running listeners and their application service, with explicit bounded shutdown.
pub struct RunningSandpost {
    pub application: Application,
    pub web_listen_address: SocketAddr,
    /// The global SMTP server's actual bound address at startup.
    pub mail_listen_address: SocketAddr,
    /// Whether SMTP sessions must authenticate on the startup listener.
    pub mail_authentication_required: bool,
    /// How sessions on the startup listener are protected in transit.
    pub mail_encryption: sandpost_application::MailListenerEncryption,
    /// The IMAP service's connection details, when IMAP is enabled.
    pub imap: Option<sandpost_application::ImapServerDetails>,
    shutdown_sender: watch::Sender<bool>,
    supervisor: Option<JoinHandle<Result<(), RuntimeError>>>,
}

impl SandpostRuntime {
    /// Initialize shared services, bind configured HTTP/SMTP/IMAP listeners, and supervise their tasks.
    /// No process signals or global tracing subscriber are installed by this operation.
    pub async fn start(configuration: Configuration) -> Result<RunningSandpost, RuntimeError> {
        configuration.validate()?;
        // Load certificates before touching storage, so a bad path fails fast.
        let mail_encryption = MailEncryption::load(&configuration)?;
        let prepared_imap = crate::imap_listener::prepare(&configuration.imap)?;
        let storage = bootstrap::initialize(&configuration).await?;
        let (event_broadcast, _) = broadcast::channel(256);
        let instance =
            Instance::open(storage.clone(), configuration.data_directory.clone()).await?;
        let index_directory = configuration.data_directory.join("search");
        let search = SearchCoordinator::open(storage.clone(), index_directory).await?;
        let mail_server = MailServer::new(
            MailDelivery {
                authenticator: SmtpAuthenticator::new(storage.clone()),
                storage: storage.clone(),
                events: event_broadcast.clone(),
            },
            configuration.mail_authentication,
            mail_encryption.clone(),
        );
        let application = Application::with_search(
            storage.clone(),
            event_broadcast.clone(),
            instance,
            search.clone(),
        )
        .with_scope_depth_limit(configuration.maximum_scope_depth)
        .with_mail_listeners(Arc::new(mail_server.clone()));
        let mail_authentication_required = authentication_policy(
            configuration.mail_authentication,
            configuration.mail_listen_address,
        ) == AuthenticationPolicy::Required;
        let startup_encryption = mail_encryption.describe(configuration.mail_listen_address);
        let (shutdown_sender, shutdown_receiver) = watch::channel(false);
        let application_router = if sandpost_web_api::EMBEDDED_FRONTEND {
            sandpost_web_api::router_with_embedded_frontend(application.clone())
        } else {
            sandpost_web_api::router(application.clone())
        }
        .layer(axum::middleware::from_fn_with_state(
            shutdown_receiver.clone(),
            http::cancel_on_shutdown,
        ));
        let web_listener = TcpListener::bind(configuration.web_listen_address).await?;
        let web_listen_address = web_listener.local_addr()?;
        let imap =
            crate::imap_listener::bind(&configuration.imap, prepared_imap, &application).await?;
        // The SMTP listener starts serving immediately, so it binds after every fallible step.
        let mail_listen_address = mail_server.start(configuration.mail_listen_address).await?;
        tracing::debug!(web = %web_listen_address, mail = %mail_listen_address, "listeners initialized");
        let web_shutdown_receiver = shutdown_receiver.clone();
        let web_server_task = tokio::spawn(async move {
            axum::serve(web_listener, application_router)
                .with_graceful_shutdown(wait_for_shutdown(web_shutdown_receiver))
                .await
        });
        let mail_shutdown = shutdown_receiver.clone();
        let mail_server_task = tokio::spawn(async move {
            wait_for_shutdown(mail_shutdown).await;
            mail_server.shutdown().await;
            Ok::<(), std::io::Error>(())
        });
        let imap_details = imap.as_ref().map(|bound| bound.details.clone());
        let imap_shutdown = shutdown_receiver.clone();
        let imap_server_task = tokio::spawn(async move {
            match imap {
                Some(bound) => {
                    bound
                        .server
                        .serve(bound.listener, wait_for_shutdown(imap_shutdown))
                        .await
                }
                None => wait_for_shutdown(imap_shutdown).await,
            }
            Ok::<(), std::io::Error>(())
        });
        let indexing_task =
            tokio::spawn(indexing_worker(search.clone(), shutdown_receiver.clone()));
        let supervision_shutdown = shutdown_sender.clone();
        let supervisor = tokio::spawn(async move {
            let (result, shutdown_deadline) = supervise(
                web_server_task,
                mail_server_task,
                imap_server_task,
                indexing_task,
                supervision_shutdown.clone(),
                supervision_shutdown.subscribe(),
            )
            .await;
            let runtime = tokio::runtime::Handle::current();
            let close_result = run_shutdown_cleanup(
                move || runtime.block_on(search.close_writer()).map_err(Into::into),
                shutdown_deadline,
            )
            .await;
            result.and(close_result)
        });
        Ok(RunningSandpost {
            application,
            web_listen_address,
            mail_listen_address,
            mail_authentication_required,
            mail_encryption: startup_encryption,
            imap: imap_details,
            shutdown_sender,
            supervisor: Some(supervisor),
        })
    }
}

impl RunningSandpost {
    /// Request transport shutdown without blocking the caller.
    pub fn request_shutdown(&self) {
        let _ = self.shutdown_sender.send(true);
    }

    /// Observe service termination; cancellation leaves supervision available for shutdown.
    pub async fn wait(&mut self) -> Result<(), RuntimeError> {
        if let Some(supervisor) = self.supervisor.as_mut() {
            let result = supervisor.await;
            self.supervisor = None;
            result?
        } else {
            Ok(())
        }
    }

    /// Stop listeners and wait at most the shared drain deadline for active transport tasks.
    pub async fn shutdown(mut self) -> Result<(), RuntimeError> {
        self.request_shutdown();
        self.wait().await
    }
}

impl Drop for RunningSandpost {
    /// Request bounded background draining if the host drops its running handle.
    fn drop(&mut self) {
        self.request_shutdown();
    }
}

#[cfg(test)]
mod tests;
