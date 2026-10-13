//! Parsed client commands (RFC 3501 section 6 and the supported extensions).
use crate::protocol::{datetime::CivilDate, flags::FlagChange, sequence::SequenceSet};

/// One complete client command: its tag and what it asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub tag: String,
    pub kind: CommandKind,
}

/// The command and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandKind {
    Capability,
    Noop,
    Logout,
    StartTls,
    /// `AUTHENTICATE mechanism [initial-response]` (RFC 3501, RFC 4959 SASL-IR).
    Authenticate {
        mechanism: String,
        initial_response: Option<String>,
    },
    Login {
        username: String,
        password: String,
    },
    /// SELECT (`read_only` false) or EXAMINE (`read_only` true).
    Select {
        mailbox: String,
        read_only: bool,
    },
    Create {
        mailbox: String,
    },
    Delete {
        mailbox: String,
    },
    Rename {
        source: String,
        destination: String,
    },
    /// SUBSCRIBE (`subscribe` true) or UNSUBSCRIBE.
    Subscribe {
        mailbox: String,
        subscribe: bool,
    },
    /// LIST or, with `subscribed_only`, LSUB.
    List {
        reference: String,
        pattern: String,
        subscribed_only: bool,
    },
    Namespace,
    Status {
        mailbox: String,
        items: Vec<StatusItem>,
    },
    /// APPEND whose message literal the server accepted; the content is not retained.
    Append {
        mailbox: String,
    },
    Check,
    Close,
    Unselect,
    /// EXPUNGE, or UID EXPUNGE restricted to a UID set (RFC 4315).
    Expunge {
        uids: Option<SequenceSet>,
    },
    Search {
        uid: bool,
        charset: Option<String>,
        criteria: SearchKey,
    },
    Fetch {
        uid: bool,
        set: SequenceSet,
        attributes: Vec<FetchAttribute>,
    },
    Store {
        uid: bool,
        set: SequenceSet,
        change: FlagChange,
        silent: bool,
    },
    /// COPY, or MOVE (RFC 6851) when `remove_from_source` is set.
    Copy {
        uid: bool,
        set: SequenceSet,
        mailbox: String,
        remove_from_source: bool,
    },
    Idle,
    /// ID (RFC 2971); the client's parameters are accepted and ignored.
    Id,
}

impl CommandKind {
    /// Defer EXPUNGE responses when renumbering would change the command's message targets.
    ///
    /// RFC 3501 section 7.4.1 requires deferral for ordinary FETCH, STORE, and SEARCH. COPY,
    /// MOVE, and UID SEARCH also retain their received sequence positions until resolution;
    /// all SEARCH commands defer removals because even UID SEARCH can contain sequence keys.
    pub fn forbids_expunge_responses(&self) -> bool {
        matches!(
            self,
            Self::Fetch { uid: false, .. }
                | Self::Store { uid: false, .. }
                | Self::Search { .. }
                | Self::Copy { uid: false, .. }
        )
    }
}

/// A STATUS data item (RFC 3501 section 6.3.10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusItem {
    Messages,
    Recent,
    UidNext,
    UidValidity,
    Unseen,
}

impl StatusItem {
    /// The item name as written in a STATUS response.
    pub fn name(self) -> &'static str {
        match self {
            Self::Messages => "MESSAGES",
            Self::Recent => "RECENT",
            Self::UidNext => "UIDNEXT",
            Self::UidValidity => "UIDVALIDITY",
            Self::Unseen => "UNSEEN",
        }
    }
}

/// A FETCH data item (RFC 3501 section 6.4.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchAttribute {
    Envelope,
    Flags,
    InternalDate,
    Rfc822Size,
    Uid,
    BodyStructure,
    /// `BODY` without a section: the non-extensible body structure.
    Body,
    /// `RFC822`: the whole message; sets `\Seen`.
    Rfc822,
    /// `RFC822.HEADER`: the header; does not set `\Seen`.
    Rfc822Header,
    /// `RFC822.TEXT`: the body text; sets `\Seen`.
    Rfc822Text,
    /// `BODY[section]<partial>` or `BODY.PEEK[section]<partial>`.
    Section {
        peek: bool,
        section: Section,
        partial: Option<(u32, u32)>,
    },
}

impl FetchAttribute {
    /// Report whether fetching this item sets `\Seen` on a read-write mailbox.
    pub fn sets_seen(&self) -> bool {
        matches!(
            self,
            Self::Rfc822 | Self::Rfc822Text | Self::Section { peek: false, .. }
        )
    }

    /// Report whether this item needs the message octets.
    pub fn needs_content(&self) -> bool {
        matches!(
            self,
            Self::Envelope
                | Self::BodyStructure
                | Self::Body
                | Self::Rfc822
                | Self::Rfc822Header
                | Self::Rfc822Text
                | Self::Section { .. }
        )
    }
}

/// A body section specification: a part path and an optional text selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Part numbers, outermost first; empty for the whole message.
    pub part: Vec<u32>,
    pub text: Option<SectionText>,
}

/// What part of a message or body part a section selects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionText {
    Header,
    /// `HEADER.FIELDS (...)` or, with `not`, `HEADER.FIELDS.NOT (...)`.
    HeaderFields {
        not: bool,
        fields: Vec<String>,
    },
    Text,
    Mime,
}

/// A SEARCH criterion (RFC 3501 section 6.4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchKey {
    All,
    /// A parenthesized or top-level list: every key must match.
    And(Vec<SearchKey>),
    Answered,
    Bcc(String),
    Before(CivilDate),
    Body(String),
    Cc(String),
    Deleted,
    Draft,
    Flagged,
    From(String),
    Header(String, String),
    Keyword(String),
    Larger(u32),
    New,
    Not(Box<SearchKey>),
    Old,
    On(CivilDate),
    Or(Box<SearchKey>, Box<SearchKey>),
    Recent,
    Seen,
    SentBefore(CivilDate),
    SentOn(CivilDate),
    SentSince(CivilDate),
    Since(CivilDate),
    Smaller(u32),
    Subject(String),
    Text(String),
    To(String),
    Uid(SequenceSet),
    Unanswered,
    Undeleted,
    Undraft,
    Unflagged,
    Unkeyword(String),
    Unseen,
    /// A bare sequence set of message numbers.
    Sequence(SequenceSet),
}

impl SearchKey {
    /// Report whether evaluating this key needs message octets (headers or body).
    pub fn needs_content(&self) -> bool {
        match self {
            Self::And(keys) => keys.iter().any(Self::needs_content),
            Self::Not(key) => key.needs_content(),
            Self::Or(left, right) => left.needs_content() || right.needs_content(),
            Self::Bcc(_)
            | Self::Body(_)
            | Self::Cc(_)
            | Self::From(_)
            | Self::Header(..)
            | Self::SentBefore(_)
            | Self::SentOn(_)
            | Self::SentSince(_)
            | Self::Subject(_)
            | Self::Text(_)
            | Self::To(_) => true,
            _ => false,
        }
    }

    /// Report whether evaluating this key needs size or internal-date metadata.
    pub fn needs_metadata(&self) -> bool {
        match self {
            Self::And(keys) => keys.iter().any(Self::needs_metadata),
            Self::Not(key) => key.needs_metadata(),
            Self::Or(left, right) => left.needs_metadata() || right.needs_metadata(),
            Self::Before(_) | Self::On(_) | Self::Since(_) | Self::Larger(_) | Self::Smaller(_) => {
                true
            }
            _ => false,
        }
    }
}
