//! IDLE (RFC 2177): push mailbox changes while the client waits.
//!
//! The session waits on the client's DONE, the account's change subscription, a keepalive
//! timer, the IDLE time limit, and server shutdown at once. Change notifications are coalesced
//! briefly, then the account is revalidated and the selected mailbox synchronized, so updates
//! arrive without polling storage.
use super::{Next, Session, bounded, connection::LineRead, wait_for_shutdown};
use crate::{
    backend::{Account, BackendError, ChangeWatch, ImapBackend},
    protocol::response::{ResponseCode, Status},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    time::{Instant, sleep, sleep_until},
};

/// Longest line accepted while idling; anything but DONE ends the IDLE with BAD.
const MAXIMUM_IDLE_LINE: usize = 64;

impl<Stream, Backend> Session<Stream, Backend>
where
    Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    Backend: ImapBackend,
{
    /// Run IDLE until DONE, disconnection, revocation, the time limit, or shutdown.
    pub(super) async fn idle(&mut self, tag: &str) -> std::io::Result<Next> {
        let limits = self.limits();
        self.connection.output.continuation("idling");
        self.connection.flush().await?;
        let deadline = Instant::now() + limits.idle_timeout;
        let mut keepalive = Instant::now() + limits.idle_keepalive_interval;
        let mut notifications_open = true;
        loop {
            let event = {
                let shutdown = wait_for_shutdown(self.shutdown.clone());
                let watch = &mut self.account.as_mut().expect("authenticated").watch;
                let line = self.connection.read_line(MAXIMUM_IDLE_LINE);
                tokio::select! {
                    biased;
                    _ = shutdown => IdleEvent::Shutdown,
                    line = line => IdleEvent::Input(line?),
                    open = watch.changed(), if notifications_open => IdleEvent::Changed(open),
                    _ = sleep_until(keepalive) => IdleEvent::Keepalive,
                    _ = sleep_until(deadline) => IdleEvent::Expired,
                }
            };
            match event {
                IdleEvent::Input(LineRead::Line(line)) if line.eq_ignore_ascii_case(b"DONE") => {
                    break;
                }
                IdleEvent::Input(LineRead::Line(_)) | IdleEvent::Input(LineRead::TooLong) => {
                    self.connection
                        .output
                        .status(Some(tag), Status::Bad, None, "Expected DONE");
                    return Ok(Next::Continue);
                }
                IdleEvent::Input(LineRead::Closed) => return Ok(Next::Close),
                IdleEvent::Shutdown => {
                    self.connection
                        .output
                        .status(None, Status::Bye, None, "Server shutting down");
                    return Ok(Next::Close);
                }
                IdleEvent::Expired => {
                    self.connection.output.status(
                        None,
                        Status::Bye,
                        None,
                        "IDLE timed out; please reconnect",
                    );
                    return Ok(Next::Close);
                }
                IdleEvent::Keepalive => {
                    self.connection
                        .output
                        .status(None, Status::Ok, None, "Still here");
                    keepalive = Instant::now() + limits.idle_keepalive_interval;
                }
                IdleEvent::Changed(open) => {
                    notifications_open = open;
                    if open {
                        // Let a burst of related changes settle into one synchronization.
                        sleep(limits.change_coalescing_delay).await;
                    }
                    if let Some(next) = self.refresh_while_idle().await {
                        return Ok(next);
                    }
                }
            }
            self.connection.flush().await?;
        }
        if let Some(next) = self.refresh_while_idle().await {
            return Ok(next);
        }
        self.ok(tag, "IDLE terminated");
        Ok(Next::Continue)
    }

    /// Revalidate and synchronize, closing on failure so a consumed change is never lost silently.
    async fn refresh_while_idle(&mut self) -> Option<Next> {
        let limit = self.limits().backend_timeout;
        match bounded(limit, self.account().validate()).await {
            Ok(Ok(())) => {}
            Ok(Err(BackendError::AccountRevoked)) | Err(BackendError::AccountRevoked) => {
                return Some(self.revoked());
            }
            Ok(Err(BackendError::Unavailable(_))) | Err(BackendError::Unavailable(_)) => {
                return Some(self.idle_unavailable());
            }
        }
        match self.synchronize(true).await {
            Err(crate::backend::MailboxError::Backend(BackendError::AccountRevoked)) => {
                Some(self.revoked())
            }
            Ok(()) => None,
            Err(_) => Some(self.idle_unavailable()),
        }
    }

    /// End IDLE after a refresh failure; reconnecting lets the client resynchronize safely.
    fn idle_unavailable(&mut self) -> Next {
        self.connection.output.status(
            None,
            Status::Bye,
            Some(&ResponseCode::Unavailable),
            "Mailbox updates unavailable; please reconnect",
        );
        Next::Close
    }
}

/// What woke an idling session.
enum IdleEvent {
    Input(LineRead),
    Changed(bool),
    Keepalive,
    Expired,
    Shutdown,
}
