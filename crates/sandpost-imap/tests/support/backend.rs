//! An in-memory reference backend: mailboxes with persistent UIDs and flags, a Trash that
//! accepts copies, change notifications, and account revocation.
use sandpost_imap::{
    Account, AuthenticationOutcome, BackendError, ChangeWatch, CopyOutcome, Credentials,
    FlagChange, Flags, ImapBackend, MailboxChanges, MailboxError, MailboxListing, MailboxSnapshot,
    MailboxStatus, MessageContent, MessageMetadata, MessageState, SpecialUse,
    SynchronizationRequest, SystemFlag, Uid,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;

/// The only username the backend accepts.
pub const USERNAME: &str = "developer";
/// The only password the backend accepts.
pub const PASSWORD: &str = "correct horse battery";

/// One stored message.
#[derive(Clone)]
struct StoredMessage {
    octets: Arc<[u8]>,
    internal_date: i64,
    flags: Flags,
    /// The modification at which the flags last changed.
    modified: u64,
}

/// One mailbox.
struct StoredMailbox {
    uid_validity: u32,
    uid_next: Uid,
    special_use: Option<SpecialUse>,
    messages: BTreeMap<Uid, StoredMessage>,
    /// Highest UID already reported as recent to some read-write session.
    recent_through: Uid,
}

/// All backend state behind one lock.
struct State {
    enabled: bool,
    mailboxes: BTreeMap<String, StoredMailbox>,
    unsubscribed: BTreeSet<String>,
    modification: u64,
}

/// Cloneable handle to the shared in-memory store.
#[derive(Clone)]
pub struct MemoryBackend {
    state: Arc<Mutex<State>>,
    changes: broadcast::Sender<()>,
}

impl MemoryBackend {
    /// An enabled account with INBOX, Laravel, and Trash.
    pub fn new() -> Self {
        let mut mailboxes = BTreeMap::new();
        for (index, (name, special_use)) in [
            ("INBOX", None),
            ("Laravel", None),
            ("Trash", Some(SpecialUse::Trash)),
            ("Caf\u{e9}", None),
        ]
        .into_iter()
        .enumerate()
        {
            mailboxes.insert(
                name.to_owned(),
                StoredMailbox {
                    uid_validity: 1000 + index as u32,
                    uid_next: 1,
                    special_use,
                    messages: BTreeMap::new(),
                    recent_through: 0,
                },
            );
        }
        let (changes, _) = broadcast::channel(64);
        Self {
            state: Arc::new(Mutex::new(State {
                enabled: true,
                mailboxes,
                unsubscribed: BTreeSet::new(),
                modification: 1,
            })),
            changes,
        }
    }

    /// Store a message in a mailbox and notify watchers; returns its UID.
    pub fn deliver(&self, mailbox: &str, octets: &[u8], internal_date: i64) -> Uid {
        let uid = {
            let mut state = self.state.lock().unwrap();
            state.modification += 1;
            let modification = state.modification;
            let mailbox = state.mailboxes.get_mut(mailbox).expect("mailbox exists");
            let uid = mailbox.uid_next;
            mailbox.uid_next += 1;
            mailbox.messages.insert(
                uid,
                StoredMessage {
                    octets: Arc::from(octets),
                    internal_date,
                    flags: Flags::new(),
                    modified: modification,
                },
            );
            uid
        };
        let _ = self.changes.send(());
        uid
    }

    /// Remove a message as another client or a permission change would, and notify watchers.
    pub fn remove(&self, mailbox: &str, uid: Uid) {
        self.state
            .lock()
            .unwrap()
            .mailboxes
            .get_mut(mailbox)
            .expect("mailbox exists")
            .messages
            .remove(&uid);
        let _ = self.changes.send(());
    }

    /// Replace a message's flags as another session would, and notify watchers.
    pub fn set_flags(&self, mailbox: &str, uid: Uid, flags: Flags) {
        {
            let mut state = self.state.lock().unwrap();
            state.modification += 1;
            let modification = state.modification;
            let message = state
                .mailboxes
                .get_mut(mailbox)
                .unwrap()
                .messages
                .get_mut(&uid)
                .unwrap();
            message.flags = flags;
            message.modified = modification;
        }
        let _ = self.changes.send(());
    }

    /// Disable the account and notify watchers.
    pub fn revoke(&self) {
        self.state.lock().unwrap().enabled = false;
        let _ = self.changes.send(());
    }

    /// Delete a mailbox entirely, as when its source definition disappears.
    pub fn delete_mailbox(&self, name: &str) {
        self.state.lock().unwrap().mailboxes.remove(name);
        let _ = self.changes.send(());
    }

    /// Current flags of one stored message.
    pub fn flags(&self, mailbox: &str, uid: Uid) -> Option<Flags> {
        let state = self.state.lock().unwrap();
        state
            .mailboxes
            .get(mailbox)?
            .messages
            .get(&uid)
            .map(|message| message.flags.clone())
    }

    /// UIDs currently stored in a mailbox.
    pub fn uids(&self, mailbox: &str) -> Vec<Uid> {
        self.state.lock().unwrap().mailboxes[mailbox]
            .messages
            .keys()
            .copied()
            .collect()
    }
}

impl ImapBackend for MemoryBackend {
    type Account = MemoryAccount;

    /// Accept the fixed credential while the account is enabled.
    async fn authenticate(
        &self,
        credentials: Credentials,
    ) -> Result<AuthenticationOutcome<MemoryAccount>, BackendError> {
        if credentials.username == "throttled" {
            return Ok(AuthenticationOutcome::Throttled);
        }
        if credentials.username == "unavailable" {
            return Err(BackendError::Unavailable("storage offline".into()));
        }
        let enabled = self.state.lock().unwrap().enabled;
        Ok(
            if enabled && credentials.username == USERNAME && credentials.password == PASSWORD {
                AuthenticationOutcome::Authenticated(MemoryAccount(self.clone()))
            } else {
                AuthenticationOutcome::InvalidCredentials
            },
        )
    }
}

/// The authenticated account; all accounts share the one store.
pub struct MemoryAccount(MemoryBackend);

/// A broadcast subscription used while idling.
pub struct MemoryWatch(broadcast::Receiver<()>);

impl ChangeWatch for MemoryWatch {
    /// Wait for the next notification; lagging counts as a change.
    async fn changed(&mut self) -> bool {
        match self.0.recv().await {
            Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => true,
            Err(broadcast::error::RecvError::Closed) => false,
        }
    }
}

impl MemoryAccount {
    /// Lock the state, refusing a disabled account.
    fn state(&self) -> Result<std::sync::MutexGuard<'_, State>, BackendError> {
        let state = self.0.state.lock().unwrap();
        if state.enabled {
            Ok(state)
        } else {
            Err(BackendError::AccountRevoked)
        }
    }
}

impl Account for MemoryAccount {
    type Mailbox = String;
    type Watch = MemoryWatch;

    async fn validate(&self) -> Result<(), BackendError> {
        self.state().map(|_| ())
    }

    async fn mailboxes(&self) -> Result<Vec<MailboxListing>, BackendError> {
        let state = self.state()?;
        Ok(state
            .mailboxes
            .iter()
            .map(|(name, mailbox)| MailboxListing {
                name: name.clone(),
                selectable: true,
                special_use: mailbox.special_use,
                subscribed: !state.unsubscribed.contains(name),
            })
            .collect())
    }

    async fn set_subscription(&self, name: &str, subscribed: bool) -> Result<(), MailboxError> {
        let mut state = self.state()?;
        if subscribed {
            state.unsubscribed.remove(name);
        } else {
            state.unsubscribed.insert(name.to_owned());
        }
        Ok(())
    }

    async fn open(
        &self,
        name: &str,
        read_only: bool,
    ) -> Result<MailboxSnapshot<String>, MailboxError> {
        let mut state = self.state()?;
        let modification = state.modification;
        let mailbox = state
            .mailboxes
            .get_mut(name)
            .ok_or(MailboxError::NonExistent)?;
        let recent_through = mailbox.recent_through;
        if !read_only {
            mailbox.recent_through = mailbox.uid_next - 1;
        }
        Ok(MailboxSnapshot {
            handle: name.to_owned(),
            uid_validity: mailbox.uid_validity,
            uid_next: mailbox.uid_next,
            messages: mailbox
                .messages
                .iter()
                .map(|(uid, message)| MessageState {
                    uid: *uid,
                    flags: message.flags.clone(),
                    recent: *uid > recent_through,
                })
                .collect(),
            seen_only: false,
            read_only: false,
            cursor: modification,
        })
    }

    async fn status(&self, name: &str) -> Result<MailboxStatus, MailboxError> {
        let state = self.state()?;
        let mailbox = state.mailboxes.get(name).ok_or(MailboxError::NonExistent)?;
        Ok(MailboxStatus {
            messages: mailbox.messages.len() as u32,
            recent: mailbox
                .messages
                .keys()
                .filter(|uid| **uid > mailbox.recent_through)
                .count() as u32,
            uid_next: mailbox.uid_next,
            uid_validity: mailbox.uid_validity,
            unseen: mailbox
                .messages
                .values()
                .filter(|message| !message.flags.contains(SystemFlag::Seen))
                .count() as u32,
        })
    }

    async fn synchronize(
        &self,
        mailbox: &String,
        request: SynchronizationRequest,
    ) -> Result<MailboxChanges, MailboxError> {
        let mut state = self.state()?;
        let modification = state.modification;
        let stored = state
            .mailboxes
            .get_mut(mailbox)
            .ok_or(MailboxError::NonExistent)?;
        let known: Vec<Uid> = stored
            .messages
            .range(..=request.highest_uid)
            .map(|(uid, _)| *uid)
            .collect();
        let present_uids = (known.len() as u32 != request.known_count).then(|| known.clone());
        let recent_through = stored.recent_through;
        if request.claim_recent {
            stored.recent_through = stored.uid_next - 1;
        }
        Ok(MailboxChanges {
            present_uids,
            added: stored
                .messages
                .range(request.highest_uid + 1..)
                .map(|(uid, message)| MessageState {
                    uid: *uid,
                    flags: message.flags.clone(),
                    recent: request.claim_recent && *uid > recent_through,
                })
                .collect(),
            flag_updates: stored
                .messages
                .range(..=request.highest_uid)
                .filter(|(_, message)| message.modified > request.cursor)
                .map(|(uid, message)| (*uid, message.flags.clone()))
                .collect(),
            uid_next: stored.uid_next,
            cursor: modification,
            read_only: false,
        })
    }

    async fn metadata(
        &self,
        mailbox: &String,
        uids: &[Uid],
    ) -> Result<Vec<MessageMetadata>, MailboxError> {
        let state = self.state()?;
        let stored = state
            .mailboxes
            .get(mailbox)
            .ok_or(MailboxError::NonExistent)?;
        Ok(uids
            .iter()
            .filter_map(|uid| {
                stored.messages.get(uid).map(|message| MessageMetadata {
                    uid: *uid,
                    size: message.octets.len() as u64,
                    internal_date: message.internal_date,
                })
            })
            .collect())
    }

    async fn content(
        &self,
        mailbox: &String,
        uid: Uid,
    ) -> Result<Option<MessageContent>, MailboxError> {
        let state = self.state()?;
        let stored = state
            .mailboxes
            .get(mailbox)
            .ok_or(MailboxError::NonExistent)?;
        Ok(stored.messages.get(&uid).map(|message| MessageContent {
            octets: message.octets.clone(),
            internal_date: message.internal_date,
        }))
    }

    async fn store(
        &self,
        mailbox: &String,
        uids: &[Uid],
        change: &FlagChange,
    ) -> Result<Vec<(Uid, Flags)>, MailboxError> {
        let mut state = self.state()?;
        state.modification += 1;
        let modification = state.modification;
        let stored = state
            .mailboxes
            .get_mut(mailbox)
            .ok_or(MailboxError::NonExistent)?;
        let mut result = Vec::new();
        for uid in uids {
            if let Some(message) = stored.messages.get_mut(uid) {
                message.flags = message.flags.apply(change);
                message.modified = modification;
                result.push((*uid, message.flags.clone()));
            }
        }
        drop(state);
        let _ = self.0.changes.send(());
        Ok(result)
    }

    async fn expunge(
        &self,
        mailbox: &String,
        uids: Option<&[Uid]>,
    ) -> Result<Vec<Uid>, MailboxError> {
        let mut state = self.state()?;
        let stored = state
            .mailboxes
            .get_mut(mailbox)
            .ok_or(MailboxError::NonExistent)?;
        let removed: Vec<Uid> = stored
            .messages
            .iter()
            .filter(|(uid, message)| {
                message.flags.contains(SystemFlag::Deleted)
                    && uids.is_none_or(|uids| uids.contains(uid))
            })
            .map(|(uid, _)| *uid)
            .collect();
        for uid in &removed {
            stored.messages.remove(uid);
        }
        drop(state);
        let _ = self.0.changes.send(());
        Ok(removed)
    }

    async fn copy(
        &self,
        mailbox: &String,
        uids: &[Uid],
        destination: &str,
        remove_from_source: bool,
    ) -> Result<CopyOutcome, MailboxError> {
        let mut state = self.state()?;
        if destination == "Laravel" {
            return Err(MailboxError::NotPermitted(
                "Laravel is a dynamic folder".into(),
            ));
        }
        state.modification += 1;
        let modification = state.modification;
        let copied: Vec<(Uid, StoredMessage)> = {
            let source = state
                .mailboxes
                .get(mailbox)
                .ok_or(MailboxError::NonExistent)?;
            uids.iter()
                .filter_map(|uid| {
                    source
                        .messages
                        .get(uid)
                        .map(|message| (*uid, message.clone()))
                })
                .collect()
        };
        let target = state
            .mailboxes
            .get_mut(destination)
            .ok_or(MailboxError::NonExistent)?;
        let mut outcome = CopyOutcome {
            uid_validity: target.uid_validity,
            ..CopyOutcome::default()
        };
        for (uid, mut message) in copied {
            message.flags.remove(SystemFlag::Deleted);
            message.modified = modification;
            let new_uid = target.uid_next;
            target.uid_next += 1;
            target.messages.insert(new_uid, message);
            outcome.source_uids.push(uid);
            outcome.destination_uids.push(new_uid);
        }
        if remove_from_source {
            let source = state.mailboxes.get_mut(mailbox).unwrap();
            for uid in &outcome.source_uids {
                source.messages.remove(uid);
            }
            outcome.expunged = outcome.source_uids.clone();
        }
        drop(state);
        let _ = self.0.changes.send(());
        Ok(outcome)
    }

    fn watch(&self) -> MemoryWatch {
        MemoryWatch(self.0.changes.subscribe())
    }
}
