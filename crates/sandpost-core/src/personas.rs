//! Personas: user-owned virtual recipients that run a PersonaScript on their incoming mail.
use crate::{
    LinkCheckErrorKind, MessageIdentifier, PersonaExecutionIdentifier, PersonaIdentifier,
    UserIdentifier,
};
use serde::{Deserialize, Serialize};

/// A virtual recipient owned by one user.
///
/// The address is unique across the installation and stored normalized (trimmed, ASCII
/// lowercase). A Persona never grants access: it acts only on mail its owner can currently read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Persona {
    #[serde(rename = "id")]
    pub identifier: PersonaIdentifier,
    #[serde(rename = "owner_id")]
    pub owner_identifier: UserIdentifier,
    pub name: String,
    pub address: String,
    /// Optional Lucide icon name used to identify the Persona in the interface.
    #[serde(default)]
    pub icon: Option<String>,
    /// PersonaScript source, validated before it was stored.
    pub script: String,
    pub enabled: bool,
    /// Increases with definition changes other than icon-only edits, for execution recovery.
    pub revision: u64,
    /// Unix seconds, UTC.
    pub created_at: i64,
    /// Unix seconds, UTC.
    pub updated_at: i64,
}

/// The lifecycle state of one execution of a Persona's script for one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaExecutionStatus {
    /// Scheduled when the message was stored; not started yet.
    Pending,
    /// Started; actions may be in progress.
    Running,
    /// Every executed statement finished.
    Completed,
    /// An action failed, the time limit elapsed, or a restart interrupted an outbound request.
    Failed,
    /// Stopped before finishing because the Persona was disabled or deleted, its owner lost
    /// access to the message, or the owner can no longer run Personas.
    Cancelled,
}

impl PersonaExecutionStatus {
    /// Every status, used to parse stored names.
    pub const ALL: [Self; 5] = [
        Self::Pending,
        Self::Running,
        Self::Completed,
        Self::Failed,
        Self::Cancelled,
    ];

    /**
     * Return the stable lowercase name used in storage and transport representations.
     */
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /**
     * Parse a stable status name, rejecting unknown values.
     */
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == value)
    }

    /**
     * Report whether the execution has ended and will never change again.
     */
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// One execution of a Persona's script against one message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaExecution {
    #[serde(rename = "id")]
    pub identifier: PersonaExecutionIdentifier,
    /// Increasing insertion order, used for newest-first keyset pagination.
    #[serde(rename = "seq")]
    pub sequence: u64,
    #[serde(rename = "persona_id")]
    pub persona_identifier: PersonaIdentifier,
    #[serde(rename = "message_id")]
    pub message_identifier: MessageIdentifier,
    pub status: PersonaExecutionStatus,
    /// Unix seconds, UTC, when the message was stored and the execution scheduled.
    pub scheduled_at: i64,
    /// Unix seconds, UTC, when the execution first started.
    pub started_at: Option<i64>,
    /// Unix seconds, UTC, when the execution reached a terminal status.
    pub completed_at: Option<i64>,
    /// The Persona revision whose script the execution runs, fixed when it starts.
    pub persona_revision: Option<u64>,
    /// Why the execution failed or was cancelled.
    pub error_message: Option<String>,
}

/// The kind of statement an execution step performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaActionKind {
    /// A condition was evaluated.
    Branch,
    Open,
    Wait,
    Click,
}

impl PersonaActionKind {
    /// Every kind, used to parse stored names.
    pub const ALL: [Self; 4] = [Self::Branch, Self::Open, Self::Wait, Self::Click];

    /**
     * Return the stable lowercase name used in storage and transport representations.
     */
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Branch => "branch",
            Self::Open => "open",
            Self::Wait => "wait",
            Self::Click => "click",
        }
    }

    /**
     * Parse a stable kind name, rejecting unknown values.
     */
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

/// The state of one execution step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonaActionStatus {
    /// Recorded before the action's effect began; it may still be in progress.
    Started,
    Succeeded,
    Failed,
    /// A restart stopped the action while it was in progress; an outbound request that may have
    /// reached its server is not repeated.
    Interrupted,
    /// The execution was cancelled while the action was in progress.
    Cancelled,
}

impl PersonaActionStatus {
    /// Every status, used to parse stored names.
    pub const ALL: [Self; 5] = [
        Self::Started,
        Self::Succeeded,
        Self::Failed,
        Self::Interrupted,
        Self::Cancelled,
    ];

    /**
     * Return the stable lowercase name used in storage and transport representations.
     */
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
            Self::Cancelled => "cancelled",
        }
    }

    /**
     * Parse a stable status name, rejecting unknown values.
     */
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == value)
    }
}

/// The outcome of one outbound HTTP request made by a Persona.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaHttpRequest {
    pub url: String,
    /// The last URL requested after redirects, when any redirect was followed.
    pub final_url: Option<String>,
    /// The final status code, or the last one seen before a failure.
    pub status_code: Option<u16>,
    pub redirect_count: u32,
    pub content_type: Option<String>,
    /// Body bytes read from the final response.
    pub received_bytes: u64,
    /// The body exceeded the size limit and reading stopped there.
    pub truncated: bool,
    /// Why no final response was obtained; absent when one was.
    pub error_kind: Option<LinkCheckErrorKind>,
    pub error_message: Option<String>,
    pub duration_milliseconds: u64,
}

/// What one execution step did, by kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PersonaActionDetail {
    /// A condition and the branch it chose.
    Branch {
        condition: String,
        outcome: bool,
    },
    /// The remote images requested while opening the message.
    Open {
        images: Vec<PersonaHttpRequest>,
        /// Requestable images beyond the per-open limit, not requested.
        omitted_images: u32,
        /// `cid:` and `data:` images shown from the message itself.
        embedded_images: u32,
        /// Image references that cannot be requested (unsupported scheme, unresolvable, ...).
        rejected_images: u32,
    },
    Wait {
        milliseconds: u64,
    },
    /// The selected link and the request that followed it.
    Click {
        selector: String,
        href: Option<String>,
        text: Option<String>,
        url: Option<String>,
        request: Option<PersonaHttpRequest>,
    },
}

impl PersonaActionDetail {
    /**
     * Return the kind of statement this detail describes.
     */
    pub fn kind(&self) -> PersonaActionKind {
        match self {
            Self::Branch { .. } => PersonaActionKind::Branch,
            Self::Open { .. } => PersonaActionKind::Open,
            Self::Wait { .. } => PersonaActionKind::Wait,
            Self::Click { .. } => PersonaActionKind::Click,
        }
    }
}

/// One recorded step of an execution.
///
/// A step is recorded as `Started` before its effect begins and updated when it ends, so a
/// restart can tell finished steps from interrupted ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaExecutionAction {
    /// Zero-based position in execution order.
    pub step: u32,
    pub status: PersonaActionStatus,
    /// 1-based script line of the statement.
    pub line: u32,
    /// 1-based script column of the statement.
    pub column: u32,
    /// Unix seconds, UTC, when the step started.
    pub started_at: i64,
    /// How long the step took, once it ended.
    pub duration_milliseconds: Option<u64>,
    pub error_message: Option<String>,
    pub detail: PersonaActionDetail,
}
