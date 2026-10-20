//! Durable, bounded background execution of Persona scripts.
//!
//! Message insertion schedules `pending` executions in the same storage transaction as the
//! message, so SMTP never waits for a Persona and no scheduled execution is lost. The dispatcher
//! wakes on message notifications, Persona changes, finished executions, and a periodic poll; it
//! starts the oldest runnable executions while fewer than the configured number are active.
//! Every execution is an independent task: its failure, panic, or slowness affects no other
//! execution and no transport.
//!
//! Each step is journaled before its effect starts and updated when it ends. After a restart, an
//! interrupted execution replays the same deterministic program: finished steps are skipped,
//! an interrupted wait sleeps only its remaining time, and an outbound request that had started
//! is marked interrupted and never repeated, failing the execution. Outbound requests are never
//! retried, because a click or a tracking pixel may not be idempotent.
use super::access::{Denial, eligibility};
use crate::credentials::current_unix_seconds;
use crate::{Application, ApplicationError};
use async_trait::async_trait;
use sandpost_core::{
    MessageIdentifier, Persona, PersonaActionDetail, PersonaActionStatus, PersonaExecution,
    PersonaExecutionAction, PersonaExecutionIdentifier, PersonaExecutionStatus, PersonaHttpRequest,
    PersonaIdentifier,
};
use sandpost_link_check::{LinkCheckSettings, LinkChecker, VisitPurpose, VisitResult};
use sandpost_persona_script::document::{remote_images, select_link};
use sandpost_persona_script::{
    ActionHandler, Condition, LinkSelector, Program, ScriptError, ScriptLimits, Step, execute,
};
use sandpost_storage::{Storage, StorageError};
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{Notify, Semaphore, broadcast, watch};
use tokio::task::JoinSet;

/// Remote images of one `open` requested at the same time.
const IMAGE_REQUESTS_PER_OPEN: usize = 4;

/// Why an execution whose outbound request was interrupted by a restart fails.
const INTERRUPTED_REQUEST: &str = "a restart interrupted this request; it is not repeated because it may already have reached the server";

/// Limits and network policy of Persona executions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaSettings {
    /// Permit requests to loopback, private, link-local and other non-public destinations.
    pub allow_internal_addresses: bool,
    /// Longest single `wait`.
    pub maximum_wait: Duration,
    /// Longest whole execution, measured from its first start, waits and requests included.
    /// Scripts whose waits along any path exceed three quarters of it are rejected.
    pub execution_timeout: Duration,
    /// Executions active at once, including those waiting.
    pub concurrent_executions: usize,
    /// Outbound requests in flight at once across every execution.
    pub concurrent_requests: usize,
    /// Bound on one request including its redirects and body.
    pub request_timeout: Duration,
    /// Body bytes read from one response.
    pub maximum_response_bytes: usize,
    /// Fallback interval at which the dispatcher looks for runnable executions.
    pub poll_interval: Duration,
}

impl Default for PersonaSettings {
    /**
     * Refuse internal destinations, allow ten-minute waits within a thirty-minute execution, run
     * 64 executions and 8 requests at once, and read at most 1 MiB per response.
     */
    fn default() -> Self {
        Self {
            allow_internal_addresses: false,
            maximum_wait: Duration::from_secs(10 * 60),
            execution_timeout: Duration::from_secs(30 * 60),
            concurrent_executions: 64,
            concurrent_requests: 8,
            request_timeout: Duration::from_secs(20),
            maximum_response_bytes: 1024 * 1024,
            poll_interval: Duration::from_secs(2),
        }
    }
}

impl PersonaSettings {
    /**
     * Return the script limits these settings impose.
     */
    pub fn script_limits(&self) -> ScriptLimits {
        ScriptLimits {
            maximum_wait: self.maximum_wait,
            maximum_total_wait: self.execution_timeout / 4 * 3,
            ..ScriptLimits::default()
        }
    }
}

/// An execution currently owned by this process.
struct ActiveExecution {
    persona: PersonaIdentifier,
    cancel: watch::Sender<Option<String>>,
}

/// Background execution shared by every transport of one application.
pub(crate) struct PersonaRuntime {
    settings: PersonaSettings,
    visitor: LinkChecker,
    active: Mutex<HashMap<PersonaExecutionIdentifier, ActiveExecution>>,
    programs: Mutex<HashMap<PersonaIdentifier, (u64, Arc<Program>)>>,
    wake: Notify,
    stopping: watch::Sender<bool>,
}

/// Why an execution stopped before finishing its program.
enum Halt {
    Failed(String),
    Cancelled(String),
    /// The runtime is shutting down; recorded state allows resuming after restart.
    Stopped,
    Storage(ApplicationError),
}

impl PersonaRuntime {
    /**
     * Create a runtime whose outbound requests follow `settings`.
     */
    pub(crate) fn new(settings: PersonaSettings) -> Self {
        let visitor = LinkChecker::new(LinkCheckSettings {
            allow_internal_addresses: settings.allow_internal_addresses,
            connect_timeout: Duration::from_secs(5).min(settings.request_timeout),
            response_timeout: Duration::from_secs(10).min(settings.request_timeout),
            link_timeout: settings.request_timeout,
            maximum_redirects: 10,
            concurrency_per_check: IMAGE_REQUESTS_PER_OPEN,
            concurrency: settings.concurrent_requests.max(1),
        });
        Self {
            settings,
            visitor,
            active: Mutex::new(HashMap::new()),
            programs: Mutex::new(HashMap::new()),
            wake: Notify::new(),
            stopping: watch::channel(false).0,
        }
    }

    /**
     * Return the limits and network policy of this runtime.
     */
    pub(crate) fn settings(&self) -> &PersonaSettings {
        &self.settings
    }

    /**
     * Compile a script under this runtime's limits.
     */
    pub(crate) fn compile(&self, script: &str) -> Result<Program, ScriptError> {
        sandpost_persona_script::compile(script, &self.settings.script_limits())
    }

    /**
     * Ask the dispatcher to look for runnable executions now.
     */
    pub(crate) fn wake(&self) {
        self.wake.notify_one();
    }

    /**
     * Stop this process's active executions of a Persona with a reason.
     */
    pub(crate) fn cancel_persona(&self, persona: PersonaIdentifier, reason: &str) {
        if let Ok(active) = self.active.lock() {
            for execution in active
                .values()
                .filter(|execution| execution.persona == persona)
            {
                execution.cancel.send_replace(Some(reason.to_owned()));
            }
        }
        if let Ok(mut programs) = self.programs.lock() {
            programs.remove(&persona);
        }
    }

    /**
     * Return the compiled program of a Persona's current revision, compiling it once.
     */
    fn program(&self, persona: &Persona) -> Result<Arc<Program>, ScriptError> {
        if let Ok(programs) = self.programs.lock()
            && let Some((revision, program)) = programs.get(&persona.identifier)
            && *revision == persona.revision
        {
            return Ok(program.clone());
        }
        let program = Arc::new(self.compile(&persona.script)?);
        if let Ok(mut programs) = self.programs.lock() {
            programs.insert(persona.identifier, (persona.revision, program.clone()));
        }
        Ok(program)
    }

    /**
     * Dispatch executions until `shutdown` turns true, then stop active executions.
     *
     * Executions interrupted by shutdown stay running in storage and resume after restart.
     */
    pub(crate) async fn run(
        self: Arc<Self>,
        application: Application,
        mut shutdown: watch::Receiver<bool>,
    ) {
        let mut events = Some(application.subscribe());
        let mut tasks = JoinSet::new();
        self.stopping.send_replace(false);
        tracing::debug!("persona dispatcher started");
        while !*shutdown.borrow() {
            if let Err(error) = self.fill(&application, &mut tasks).await {
                tracing::warn!(%error, "persona dispatcher could not read runnable executions");
            }
            tokio::select! {
                () = self.wake.notified() => {}
                received = next_event(&mut events) => {
                    if matches!(received, Err(broadcast::error::RecvError::Closed)) {
                        events = None;
                    }
                }
                () = tokio::time::sleep(self.settings.poll_interval) => {}
                Some(joined) = tasks.join_next(), if !tasks.is_empty() => {
                    if let Err(error) = joined
                        && error.is_panic()
                    {
                        tracing::error!(%error, "persona execution task panicked");
                    }
                }
                _ = shutdown.changed() => {}
            }
        }
        self.stopping.send_replace(true);
        while tasks.join_next().await.is_some() {}
        if let Ok(mut active) = self.active.lock() {
            active.clear();
        }
        tracing::debug!("persona dispatcher stopped");
    }

    /**
     * Start runnable executions while capacity remains: interrupted ones first, then pending
     * ones, each in scheduling order.
     */
    async fn fill(
        self: &Arc<Self>,
        application: &Application,
        tasks: &mut JoinSet<()>,
    ) -> Result<(), ApplicationError> {
        let maximum = self.settings.concurrent_executions.max(1);
        let active_count = self.active_count();
        if active_count >= maximum {
            return Ok(());
        }
        let window = maximum + active_count;
        let storage = &application.storage;
        let mut candidates = storage
            .list_persona_executions_in_status(PersonaExecutionStatus::Running, None, window)
            .await?;
        candidates.extend(
            storage
                .list_persona_executions_in_status(PersonaExecutionStatus::Pending, None, window)
                .await?,
        );
        for execution in candidates {
            let cancel = {
                let mut active = self.active.lock().map_err(|_| {
                    ApplicationError::Unavailable("persona state is unavailable".into())
                })?;
                if active.len() >= maximum {
                    break;
                }
                if active.contains_key(&execution.identifier) {
                    continue;
                }
                let (sender, receiver) = watch::channel(None);
                active.insert(
                    execution.identifier,
                    ActiveExecution {
                        persona: execution.persona_identifier,
                        cancel: sender,
                    },
                );
                receiver
            };
            let runtime = self.clone();
            let application = application.clone();
            tasks.spawn(async move {
                let identifier = execution.identifier;
                if let Err(error) = runtime.execute(&application, execution, cancel).await {
                    tracing::warn!(execution = %identifier, %error, "persona execution paused after a storage failure; it resumes from its recorded steps");
                }
                if let Ok(mut active) = runtime.active.lock() {
                    active.remove(&identifier);
                }
                runtime.wake();
            });
        }
        Ok(())
    }

    /**
     * Return how many executions this process currently owns.
     */
    pub(crate) fn active_count(&self) -> usize {
        self.active.lock().map(|active| active.len()).unwrap_or(0)
    }

    /**
     * Run one execution from its recorded state to a terminal status.
     *
     * Returns an error only when storage failed; the execution then stays resumable.
     */
    async fn execute(
        &self,
        application: &Application,
        execution: PersonaExecution,
        cancel: watch::Receiver<Option<String>>,
    ) -> Result<(), ApplicationError> {
        let storage = &application.storage;
        let identifier = execution.identifier;
        let first_attempt = execution.status == PersonaExecutionStatus::Pending;
        let allowed = match eligibility(
            storage,
            execution.persona_identifier,
            execution.message_identifier,
        )
        .await?
        {
            Ok(allowed) => allowed,
            Err(Denial::PersonaMissing) => return Ok(()),
            Err(Denial::MessageInaccessible) if first_attempt => {
                // The owner never could read this message: leave no trace of it.
                storage.discard_persona_execution(identifier).await?;
                tracing::debug!(execution = %identifier, "persona execution discarded: message not readable by its owner");
                return Ok(());
            }
            Err(denial) => {
                storage
                    .finish_persona_execution(
                        identifier,
                        PersonaExecutionStatus::Cancelled,
                        current_unix_seconds(),
                        Some(denial.reason()),
                    )
                    .await?;
                return Ok(());
            }
        };
        let persona = allowed.persona;
        let program = match self.program(&persona) {
            Ok(program) => program,
            Err(error) => {
                storage
                    .start_persona_execution(identifier, current_unix_seconds(), persona.revision)
                    .await?;
                storage
                    .finish_persona_execution(
                        identifier,
                        PersonaExecutionStatus::Failed,
                        current_unix_seconds(),
                        Some(&format!("the script no longer compiles: {error}")),
                    )
                    .await?;
                return Ok(());
            }
        };
        let Some(started) = storage
            .start_persona_execution(identifier, current_unix_seconds(), persona.revision)
            .await?
        else {
            return Ok(());
        };
        if started.persona_revision != Some(persona.revision) {
            storage
                .finish_persona_execution(
                    identifier,
                    PersonaExecutionStatus::Failed,
                    current_unix_seconds(),
                    Some("the Persona's script changed before the interrupted execution could resume"),
                )
                .await?;
            return Ok(());
        }
        let recorded = if first_attempt {
            HashMap::new()
        } else {
            storage
                .get_persona_execution(identifier)
                .await?
                .map(|(_, actions)| actions)
                .unwrap_or_default()
                .into_iter()
                .map(|action| (action.step, action))
                .collect()
        };
        // Steps run sequentially, so only the last recorded step can be unfinished. An outbound
        // request it started may have reached its server: fail without repeating it, whatever
        // the remaining time.
        if let Some(interrupted) = recorded.values().find(|action| {
            action.status == PersonaActionStatus::Started
                && matches!(
                    action.detail,
                    PersonaActionDetail::Open { .. } | PersonaActionDetail::Click { .. }
                )
        }) {
            let mut action = interrupted.clone();
            action.status = PersonaActionStatus::Interrupted;
            action.error_message = Some(INTERRUPTED_REQUEST.to_owned());
            storage
                .record_persona_execution_action(identifier, &action)
                .await?;
            storage
                .finish_persona_execution(
                    identifier,
                    PersonaExecutionStatus::Failed,
                    current_unix_seconds(),
                    Some(INTERRUPTED_REQUEST),
                )
                .await?;
            return Ok(());
        }
        let elapsed = u64::try_from(
            current_unix_seconds() - started.started_at.unwrap_or_else(current_unix_seconds),
        )
        .unwrap_or(0);
        let remaining = self
            .settings
            .execution_timeout
            .saturating_sub(Duration::from_secs(elapsed));
        let mut handler = ExecutionHandler {
            runtime: self,
            storage,
            execution: identifier,
            persona: persona.identifier,
            message: execution.message_identifier,
            markup: allowed.facts.markup_body.clone(),
            recorded,
            cancel,
            stopping: self.stopping.subscribe(),
        };
        tracing::debug!(execution = %identifier, persona = %persona.identifier, message = %execution.message_identifier, resumed = !first_attempt, "persona execution started");
        let outcome =
            tokio::time::timeout(remaining, execute(&program, &allowed.facts, &mut handler)).await;
        let (status, error) = match outcome {
            Ok(Ok(report)) => {
                tracing::info!(execution = %identifier, persona = %persona.identifier, steps = report.steps, "persona execution completed");
                (PersonaExecutionStatus::Completed, None)
            }
            Ok(Err(Halt::Failed(message))) => {
                tracing::info!(execution = %identifier, persona = %persona.identifier, reason = %message, "persona execution failed");
                (PersonaExecutionStatus::Failed, Some(message))
            }
            Ok(Err(Halt::Cancelled(reason))) => {
                tracing::info!(execution = %identifier, persona = %persona.identifier, %reason, "persona execution cancelled");
                (PersonaExecutionStatus::Cancelled, Some(reason))
            }
            Ok(Err(Halt::Stopped)) => return Ok(()),
            Ok(Err(Halt::Storage(error))) => return Err(error),
            Err(_) => (
                PersonaExecutionStatus::Failed,
                Some(format!(
                    "the execution exceeded its time limit of {} seconds",
                    self.settings.execution_timeout.as_secs()
                )),
            ),
        };
        storage
            .finish_persona_execution(identifier, status, current_unix_seconds(), error.as_deref())
            .await?;
        Ok(())
    }
}

/**
 * Receive the next message notification, or never when notifications have closed.
 */
async fn next_event(
    events: &mut Option<broadcast::Receiver<crate::MessageEvent>>,
) -> Result<crate::MessageEvent, broadcast::error::RecvError> {
    match events {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

/// Performs one execution's actions with journaling, revalidation, and cancellation.
struct ExecutionHandler<'runtime> {
    runtime: &'runtime PersonaRuntime,
    storage: &'runtime Arc<dyn Storage>,
    execution: PersonaExecutionIdentifier,
    persona: PersonaIdentifier,
    message: MessageIdentifier,
    markup: String,
    /// Steps recorded before an interruption, by step number.
    recorded: HashMap<u32, PersonaExecutionAction>,
    cancel: watch::Receiver<Option<String>>,
    stopping: watch::Receiver<bool>,
}

/// What to do with a step that an interrupted run already recorded.
enum Replay {
    /// The step has not run yet.
    Fresh,
    /// The step finished before the interruption.
    Done,
    /// The step started but did not finish.
    Interrupted(Box<PersonaExecutionAction>),
}

impl ExecutionHandler<'_> {
    /**
     * Classify a step against the steps recorded before an interruption.
     */
    fn replay(&self, step: Step) -> Result<Replay, Halt> {
        match self.recorded.get(&step.index) {
            None => Ok(Replay::Fresh),
            Some(action) => match action.status {
                PersonaActionStatus::Succeeded => Ok(Replay::Done),
                PersonaActionStatus::Started => Ok(Replay::Interrupted(Box::new(action.clone()))),
                _ => Err(Halt::Failed(
                    "the execution's recorded steps are inconsistent".into(),
                )),
            },
        }
    }

    /**
     * Await a future unless the execution is cancelled or the runtime stops first.
     */
    async fn guarded<Output>(&self, future: impl Future<Output = Output>) -> Result<Output, Halt> {
        let mut cancel = self.cancel.clone();
        let mut stopping = self.stopping.clone();
        tokio::select! {
            biased;
            reason = cancelled(&mut cancel) => Err(Halt::Cancelled(reason)),
            () = stopped(&mut stopping) => Err(Halt::Stopped),
            output = future => Ok(output),
        }
    }

    /**
     * Confirm from current state that the Persona may still act on the message.
     */
    async fn revalidate(&self) -> Result<(), Halt> {
        match eligibility(self.storage, self.persona, self.message).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(denial)) => Err(Halt::Cancelled(denial.reason().to_owned())),
            Err(error) => Err(Halt::Storage(error)),
        }
    }

    /**
     * Persist one step; a vanished execution means its Persona was deleted.
     */
    async fn record(&self, action: &PersonaExecutionAction) -> Result<(), Halt> {
        match self
            .storage
            .record_persona_execution_action(self.execution, action)
            .await
        {
            Ok(()) => Ok(()),
            Err(StorageError::NotFound) => {
                Err(Halt::Cancelled(Denial::PersonaMissing.reason().to_owned()))
            }
            Err(error) => Err(Halt::Storage(error.into())),
        }
    }

    /**
     * Build a step record at the current time.
     */
    fn action(
        step: Step,
        status: PersonaActionStatus,
        detail: PersonaActionDetail,
    ) -> PersonaExecutionAction {
        PersonaExecutionAction {
            step: step.index,
            status,
            line: step.span.line,
            column: step.span.column,
            started_at: current_unix_seconds(),
            duration_milliseconds: None,
            error_message: None,
            detail,
        }
    }

    /**
     * Fail an outbound step interrupted by a restart without repeating its request.
     */
    async fn refuse_repeat(&self, mut action: Box<PersonaExecutionAction>) -> Result<(), Halt> {
        action.status = PersonaActionStatus::Interrupted;
        action.error_message = Some(INTERRUPTED_REQUEST.to_owned());
        self.record(&action).await?;
        Err(Halt::Failed(INTERRUPTED_REQUEST.to_owned()))
    }
}

#[async_trait]
impl ActionHandler for ExecutionHandler<'_> {
    type Halt = Halt;

    /**
     * Record a condition outcome; conditions have no external effect.
     */
    async fn branch(
        &mut self,
        step: Step,
        condition: &Condition,
        outcome: bool,
    ) -> Result<(), Halt> {
        if matches!(self.replay(step)?, Replay::Done) {
            return Ok(());
        }
        let mut action = Self::action(
            step,
            PersonaActionStatus::Succeeded,
            PersonaActionDetail::Branch {
                condition: condition.to_string(),
                outcome,
            },
        );
        action.duration_milliseconds = Some(0);
        self.record(&action).await
    }

    /**
     * Load the message's remote images as a mail client would, then record every outcome.
     */
    async fn open(&mut self, step: Step) -> Result<(), Halt> {
        match self.replay(step)? {
            Replay::Done => return Ok(()),
            Replay::Interrupted(action) => return self.refuse_repeat(action).await,
            Replay::Fresh => {}
        }
        self.revalidate().await?;
        let markup = self.markup.clone();
        let images = tokio::task::spawn_blocking(move || remote_images(&markup))
            .await
            .map_err(|error| {
                Halt::Failed(format!("the message HTML could not be read: {error}"))
            })?;
        let counts = |requests: Vec<PersonaHttpRequest>| PersonaActionDetail::Open {
            images: requests,
            omitted_images: u32::try_from(images.omitted).unwrap_or(u32::MAX),
            embedded_images: u32::try_from(images.embedded).unwrap_or(u32::MAX),
            rejected_images: u32::try_from(images.rejected).unwrap_or(u32::MAX),
        };
        let mut action = Self::action(step, PersonaActionStatus::Started, counts(Vec::new()));
        self.record(&action).await?;
        let started = Instant::now();
        let visitor = self.runtime.visitor.clone();
        let limit = self.runtime.settings.maximum_response_bytes;
        let urls = images.urls.clone();
        let requests = self
            .guarded(async move {
                let permits = Arc::new(Semaphore::new(IMAGE_REQUESTS_PER_OPEN));
                let mut requests = JoinSet::new();
                for (index, url) in urls.into_iter().enumerate() {
                    let visitor = visitor.clone();
                    let permits = permits.clone();
                    requests.spawn(async move {
                        let _permit = permits.acquire_owned().await;
                        (index, visitor.visit(&url, VisitPurpose::Image, limit).await)
                    });
                }
                let mut results = Vec::new();
                while let Some(joined) = requests.join_next().await {
                    if let Ok(result) = joined {
                        results.push(result);
                    }
                }
                results.sort_by_key(|(index, _)| *index);
                results
                    .into_iter()
                    .map(|(_, result)| http_request(result))
                    .collect::<Vec<_>>()
            })
            .await?;
        action.status = PersonaActionStatus::Succeeded;
        action.duration_milliseconds = Some(elapsed_milliseconds(started));
        action.detail = counts(requests);
        self.record(&action).await
    }

    /**
     * Wait without holding a thread, resuming only the remaining time after a restart, then
     * revalidate because access may have changed while waiting.
     */
    async fn wait(&mut self, step: Step, duration: Duration) -> Result<(), Halt> {
        let (mut action, remaining) = match self.replay(step)? {
            Replay::Done => return Ok(()),
            Replay::Interrupted(action) => {
                let elapsed = u64::try_from(current_unix_seconds() - action.started_at)
                    .unwrap_or(0)
                    .saturating_mul(1000);
                (
                    *action,
                    duration.saturating_sub(Duration::from_millis(elapsed)),
                )
            }
            Replay::Fresh => {
                let action = Self::action(
                    step,
                    PersonaActionStatus::Started,
                    PersonaActionDetail::Wait {
                        milliseconds: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
                    },
                );
                self.record(&action).await?;
                (action, duration)
            }
        };
        self.guarded(tokio::time::sleep(remaining)).await?;
        action.status = PersonaActionStatus::Succeeded;
        action.duration_milliseconds =
            Some(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX));
        self.record(&action).await?;
        self.revalidate().await
    }

    /**
     * Select the link and request it once, recording the selected link and the outcome.
     */
    async fn click(&mut self, step: Step, selector: &LinkSelector) -> Result<(), Halt> {
        match self.replay(step)? {
            Replay::Done => return Ok(()),
            Replay::Interrupted(action) => return self.refuse_repeat(action).await,
            Replay::Fresh => {}
        }
        self.revalidate().await?;
        let markup = self.markup.clone();
        let selection_input = selector.clone();
        let selection = tokio::task::spawn_blocking(move || select_link(&markup, &selection_input))
            .await
            .map_err(|error| {
                Halt::Failed(format!("the message HTML could not be read: {error}"))
            })?;
        let selected = match selection {
            Ok(selected) => selected,
            Err(error) => {
                let (href, text) = match &error {
                    sandpost_persona_script::document::LinkSelectionError::Unfollowable {
                        href,
                        text,
                        ..
                    } => (Some(href.clone()), Some(text.clone())),
                    _ => (None, None),
                };
                let mut action = Self::action(
                    step,
                    PersonaActionStatus::Failed,
                    PersonaActionDetail::Click {
                        selector: selector.to_string(),
                        href,
                        text,
                        url: None,
                        request: None,
                    },
                );
                let message = format!("{selector}: {error}");
                action.duration_milliseconds = Some(0);
                action.error_message = Some(message.clone());
                self.record(&action).await?;
                return Err(Halt::Failed(message));
            }
        };
        let detail = |request: Option<PersonaHttpRequest>| PersonaActionDetail::Click {
            selector: selector.to_string(),
            href: Some(selected.href.clone()),
            text: Some(selected.text.clone()),
            url: Some(selected.url.clone()),
            request,
        };
        let mut action = Self::action(step, PersonaActionStatus::Started, detail(None));
        self.record(&action).await?;
        let started = Instant::now();
        let visitor = self.runtime.visitor.clone();
        let limit = self.runtime.settings.maximum_response_bytes;
        let url = selected.url.clone();
        let result = self
            .guarded(async move { visitor.visit(&url, VisitPurpose::Document, limit).await })
            .await?;
        let responded = result.responded();
        let failure = result.error_message.clone();
        action.duration_milliseconds = Some(elapsed_milliseconds(started));
        action.detail = detail(Some(http_request(result)));
        if responded {
            action.status = PersonaActionStatus::Succeeded;
            self.record(&action).await
        } else {
            let message = format!(
                "the click request to {} failed: {}",
                selected.url,
                failure.unwrap_or_else(|| "no response".into())
            );
            action.status = PersonaActionStatus::Failed;
            action.error_message = Some(message.clone());
            self.record(&action).await?;
            Err(Halt::Failed(message))
        }
    }
}

/**
 * Resolve when the execution receives a cancellation reason; never when no reason can arrive.
 */
async fn cancelled(cancel: &mut watch::Receiver<Option<String>>) -> String {
    // Release the watch guard before possibly waiting forever.
    let reason = cancel
        .wait_for(Option::is_some)
        .await
        .map(|reason| reason.clone().unwrap_or_default());
    match reason {
        Ok(reason) => reason,
        Err(_) => std::future::pending().await,
    }
}

/**
 * Resolve when the runtime starts stopping; never when it cannot.
 */
async fn stopped(stopping: &mut watch::Receiver<bool>) {
    let closed = stopping.wait_for(|stopping| *stopping).await.is_err();
    if closed {
        std::future::pending::<()>().await;
    }
}

/**
 * Convert a visit into the recorded request outcome.
 */
fn http_request(result: VisitResult) -> PersonaHttpRequest {
    PersonaHttpRequest {
        url: result.url,
        final_url: result.final_url,
        status_code: result.status_code,
        redirect_count: result.redirect_count,
        content_type: result.content_type,
        received_bytes: result.received_bytes,
        truncated: result.truncated,
        error_kind: result.error_kind,
        error_message: result.error_message,
        duration_milliseconds: result.elapsed_milliseconds,
    }
}

/**
 * Return the milliseconds elapsed since `started`.
 */
fn elapsed_milliseconds(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
