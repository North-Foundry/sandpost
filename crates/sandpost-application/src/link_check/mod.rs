//! Shared, persisted verification of the links in authorized messages.
//!
//! A check runs in the background after an explicit request: it extracts the message's distinct
//! HTTP and HTTPS links, verifies them from the server, and atomically replaces the message's
//! stored results. Readers keep seeing the previous results until the replacement commits. At most
//! one check runs per message; a request for a message already being checked joins that run.
use crate::{ApplicationError, AuthorizedApplication, authorization};
use sandpost_core::{
    LinkCheckErrorKind, LinkCheckResult, LinkOutcome, MessageIdentifier, MessageLinkCheck,
};
pub use sandpost_link_check::LinkCheckSettings;
use sandpost_link_check::{ExtractedLinks, LinkChecker, extract_links, status_reason};
use sandpost_storage::{Storage, StorageError};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Semaphore, watch};

/// Messages accepted by one bulk request, matching other batch operations.
pub const MAXIMUM_BULK_LINK_CHECKS: usize = 256;

/// Messages whose checks may be queued or running at once across the instance.
const MAXIMUM_PENDING_CHECKS: usize = 1024;

/// Messages whose links are verified at the same time; links within each are bounded separately.
const CONCURRENT_MESSAGE_CHECKS: usize = 4;

/// The current link check state of one message, as every authorized reader sees it.
#[derive(Clone, Debug, Serialize)]
pub struct LinkCheckReport {
    #[serde(rename = "message_id")]
    pub message_identifier: MessageIdentifier,
    /// A check is queued or running; stored results stay until it completes.
    pub running: bool,
    /// Unix seconds, UTC, when the running check was requested.
    pub started_at: Option<i64>,
    /// Unix seconds, UTC, when the stored check finished; absent before the first check.
    pub checked_at: Option<i64>,
    /// Distinct links found in the message, in first-appearance order, whether checked or not.
    pub urls: Vec<String>,
    /// Distinct links beyond the per-message limit, which are neither listed nor verified.
    pub omitted_links: u32,
    pub summary: LinkCheckSummary,
    pub links: Vec<LinkReport>,
}

/// Counts of the stored results by outcome.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LinkCheckSummary {
    pub total: u32,
    pub healthy: u32,
    pub broken: u32,
    pub server_error: u32,
    pub failed: u32,
    pub inconclusive: u32,
}

/// One stored link result with its standard status reason.
#[derive(Clone, Debug, Serialize)]
pub struct LinkReport {
    pub url: String,
    pub outcome: LinkOutcome,
    pub status_code: Option<u16>,
    /// Registered reason phrase of `status_code`, such as `Not Found`.
    pub status_text: Option<&'static str>,
    pub final_url: Option<String>,
    pub redirects: u32,
    pub error_kind: Option<LinkCheckErrorKind>,
    pub error_message: Option<String>,
    pub checked_at: i64,
}

/// Messages selected for one bulk link check request.
#[derive(Clone, Debug, Deserialize)]
pub struct BulkLinkCheckRequest {
    pub identifiers: Vec<MessageIdentifier>,
}

/// What a bulk request did with each selected message.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BulkLinkCheckResult {
    /// Checks started by this request.
    pub started: u32,
    /// Messages already being checked, whose running check now also serves this request.
    pub already_running: u32,
    /// Duplicate, unknown, deleted, or unreadable messages that were ignored.
    pub skipped: u32,
}

/// Background link checks shared by every transport of one application.
pub(crate) struct LinkCheckCoordinator {
    checker: LinkChecker,
    /// Requested time of every queued or running check, by message.
    running: Mutex<HashMap<MessageIdentifier, i64>>,
    messages: Arc<Semaphore>,
    cancellation: watch::Sender<bool>,
}

/// Removes a message from the running set when its check ends for any reason.
struct RunningGuard {
    coordinator: Arc<LinkCheckCoordinator>,
    message: MessageIdentifier,
}

impl Drop for RunningGuard {
    /**
     * Release the message so a later request can start a new check.
     */
    fn drop(&mut self) {
        if let Ok(mut running) = self.coordinator.running.lock() {
            running.remove(&self.message);
        }
    }
}

/// Whether a start request created a new run.
enum StartOutcome {
    Started,
    AlreadyRunning,
}

impl LinkCheckCoordinator {
    /**
     * Create a coordinator verifying links with the supplied limits and destination policy.
     */
    pub(crate) fn new(settings: LinkCheckSettings) -> Self {
        Self {
            checker: LinkChecker::new(settings),
            running: Mutex::new(HashMap::new()),
            messages: Arc::new(Semaphore::new(CONCURRENT_MESSAGE_CHECKS)),
            cancellation: watch::channel(false).0,
        }
    }

    /**
     * Return when the message's queued or running check was requested.
     */
    fn running_since(&self, message: MessageIdentifier) -> Result<Option<i64>, ApplicationError> {
        Ok(self
            .running
            .lock()
            .map_err(|_| ApplicationError::Unavailable("link check state is unavailable".into()))?
            .get(&message)
            .copied())
    }

    /**
     * Start a background check of a message's links unless one is already queued or running.
     *
     * The message's bodies are passed in by the caller after authorization; the run never reads
     * the message again, so later visibility changes cannot widen what it inspects.
     */
    fn start(
        self: &Arc<Self>,
        storage: Arc<dyn Storage>,
        message: MessageIdentifier,
        markup: String,
        text: String,
    ) -> Result<StartOutcome, ApplicationError> {
        if *self.cancellation.borrow() {
            return Err(ApplicationError::Unavailable(
                "link checks are shutting down".into(),
            ));
        }
        {
            let mut running = self.running.lock().map_err(|_| {
                ApplicationError::Unavailable("link check state is unavailable".into())
            })?;
            if running.contains_key(&message) {
                return Ok(StartOutcome::AlreadyRunning);
            }
            if running.len() >= MAXIMUM_PENDING_CHECKS {
                return Err(ApplicationError::Unavailable(
                    "too many link checks are already in progress; try again later".into(),
                ));
            }
            running.insert(message, now());
        }
        let guard = RunningGuard {
            coordinator: self.clone(),
            message,
        };
        let coordinator = self.clone();
        let mut cancelled = self.cancellation.subscribe();
        tokio::spawn(async move {
            let _guard = guard;
            tokio::select! {
                _ = cancelled.wait_for(|cancelled| *cancelled) => {
                    tracing::debug!(%message, "link check cancelled");
                }
                () = coordinator.run(storage, message, markup, text) => {}
            }
        });
        Ok(StartOutcome::Started)
    }

    /**
     * Verify a message's links and replace its stored results.
     */
    async fn run(
        &self,
        storage: Arc<dyn Storage>,
        message: MessageIdentifier,
        markup: String,
        text: String,
    ) {
        let Ok(_permit) = self.messages.acquire().await else {
            return;
        };
        let extracted =
            match tokio::task::spawn_blocking(move || extract_links(&markup, &text)).await {
                Ok(extracted) => extracted,
                Err(error) => {
                    tracing::warn!(%message, %error, "link extraction failed");
                    return;
                }
            };
        let links = self.checker.check_links(&extracted.urls).await;
        let check = MessageLinkCheck {
            message_identifier: message,
            checked_at: now(),
            omitted_link_count: u32::try_from(extracted.omitted).unwrap_or(u32::MAX),
            links,
        };
        match storage.replace_message_link_check(&check).await {
            Ok(_) => tracing::debug!(%message, links = check.links.len(), "link check stored"),
            Err(StorageError::NotFound) => {
                tracing::debug!(%message, "message disappeared before its link check was stored");
            }
            Err(error) => tracing::warn!(%message, %error, "link check could not be stored"),
        }
    }

    /**
     * Stop queued and running checks; their previous stored results remain unchanged.
     */
    pub(crate) fn cancel(&self) {
        self.cancellation.send_replace(true);
    }
}

impl crate::Application {
    /**
     * Replace the link check limits and destination policy used by later checks.
     */
    pub fn with_link_check_settings(mut self, settings: LinkCheckSettings) -> Self {
        self.link_checks = Arc::new(LinkCheckCoordinator::new(settings));
        self
    }

    /**
     * Cancel background link checks during shutdown, leaving stored results unchanged.
     */
    pub fn cancel_link_checks(&self) {
        self.link_checks.cancel();
    }
}

impl AuthorizedApplication {
    /**
     * Read the stored and running link check state of an authorized message.
     *
     * Reading never starts or refreshes a check. Any role may read results of mail it can read.
     */
    pub async fn get_link_check(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<LinkCheckReport, ApplicationError> {
        self.account().await?;
        let (markup, text) = self.authorized_bodies(identifier).await?;
        let found = found_links(markup, text).await?;
        self.link_check_report(identifier, found).await
    }

    /**
     * Start, or join, a check of an authorized message's links and return its state.
     *
     * Every reader may start a check. Previous results remain readable until the new check
     * replaces them.
     */
    pub async fn start_link_check(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<LinkCheckReport, ApplicationError> {
        self.account().await?;
        let (markup, text) = self.authorized_bodies(identifier).await?;
        let found = found_links(markup.clone(), text.clone()).await?;
        self.application.link_checks.start(
            self.application.storage.clone(),
            identifier,
            markup,
            text,
        )?;
        self.link_check_report(identifier, found).await
    }

    /**
     * Start checks of several authorized messages, ignoring unreadable or duplicate selections.
     *
     * Every reader may start checks. Messages already being checked are not checked twice.
     * Runs share the application's message and link concurrency limits.
     */
    pub async fn start_bulk_link_checks(
        &self,
        request: BulkLinkCheckRequest,
    ) -> Result<BulkLinkCheckResult, ApplicationError> {
        self.account().await?;
        if request.identifiers.is_empty() || request.identifiers.len() > MAXIMUM_BULK_LINK_CHECKS {
            return Err(ApplicationError::InvalidInput(format!(
                "select between 1 and {MAXIMUM_BULK_LINK_CHECKS} messages"
            )));
        }
        let mut result = BulkLinkCheckResult::default();
        let mut seen = HashSet::new();
        for identifier in request.identifiers {
            if !seen.insert(identifier) {
                result.skipped += 1;
                continue;
            }
            match self.start_authorized_link_check(identifier).await {
                Ok(StartOutcome::Started) => result.started += 1,
                Ok(StartOutcome::AlreadyRunning) => result.already_running += 1,
                Err(ApplicationError::NotFound) => result.skipped += 1,
                Err(error) => return Err(error),
            }
        }
        Ok(result)
    }

    /**
     * Authorize a message against one SQL snapshot and start its check from those bodies.
     */
    async fn start_authorized_link_check(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<StartOutcome, ApplicationError> {
        let (markup, text) = self.authorized_bodies(identifier).await?;
        self.application.link_checks.start(
            self.application.storage.clone(),
            identifier,
            markup,
            text,
        )
    }

    /**
     * Return the markup and text bodies of a message authorized against one SQL snapshot.
     *
     * Deleted, unknown and unreadable messages are all not found.
     */
    async fn authorized_bodies(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<(String, String), ApplicationError> {
        let storage = &self.application.storage;
        let (facts, _) = storage
            .get_user_message_metadata(self.user_identifier, identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        authorization::require_snapshot(storage, self.user_identifier, &facts).await?;
        Ok((facts.markup_body, facts.text))
    }

    /**
     * Combine the stored check with the in-progress state of an already authorized message.
     */
    async fn link_check_report(
        &self,
        identifier: MessageIdentifier,
        found: ExtractedLinks,
    ) -> Result<LinkCheckReport, ApplicationError> {
        let started_at = self.application.link_checks.running_since(identifier)?;
        let stored = self
            .application
            .storage
            .get_message_link_check(identifier)
            .await?;
        let mut summary = LinkCheckSummary::default();
        let links = stored
            .as_ref()
            .map(|check| {
                check
                    .links
                    .iter()
                    .map(|result| {
                        count_outcome(&mut summary, result.outcome);
                        link_report(result)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(LinkCheckReport {
            message_identifier: identifier,
            running: started_at.is_some(),
            started_at,
            checked_at: stored.as_ref().map(|check| check.checked_at),
            urls: found.urls,
            omitted_links: u32::try_from(found.omitted).unwrap_or(u32::MAX),
            summary,
            links,
        })
    }
}

/**
 * List a message's distinct links without checking them, on a blocking thread.
 */
async fn found_links(markup: String, text: String) -> Result<ExtractedLinks, ApplicationError> {
    Ok(tokio::task::spawn_blocking(move || extract_links(&markup, &text)).await?)
}

/**
 * Add one stored outcome to the summary counts.
 */
fn count_outcome(summary: &mut LinkCheckSummary, outcome: LinkOutcome) {
    summary.total += 1;
    match outcome {
        LinkOutcome::Healthy => summary.healthy += 1,
        LinkOutcome::Broken => summary.broken += 1,
        LinkOutcome::ServerError => summary.server_error += 1,
        LinkOutcome::Failed => summary.failed += 1,
        LinkOutcome::Inconclusive => summary.inconclusive += 1,
    }
}

/**
 * Present one stored result with the registered reason of its status code.
 */
fn link_report(result: &LinkCheckResult) -> LinkReport {
    LinkReport {
        url: result.url.clone(),
        outcome: result.outcome,
        status_code: result.status_code,
        status_text: result.status_code.and_then(status_reason),
        final_url: result.final_url.clone(),
        redirects: result.redirect_count,
        error_kind: result.error_kind,
        error_message: result.error_message.clone(),
        checked_at: result.checked_at,
    }
}

/**
 * Return the current Unix time in whole seconds.
 */
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
