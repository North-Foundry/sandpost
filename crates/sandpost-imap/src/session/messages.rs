//! Selected-message mutations: STORE, COPY, MOVE, EXPUNGE, and their UID forms.
use super::{Next, Session, bounded, selected::Resolution};
use crate::{
    backend::{Account, ImapBackend, MailboxError, Uid},
    protocol::{
        flags::FlagChange,
        response::{ResponseCode, Status},
        sequence::{SequenceSet, format_ordered_set},
    },
};
use tokio::io::{AsyncRead, AsyncWrite};

impl<Stream, Backend> Session<Stream, Backend>
where
    Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    Backend: ImapBackend,
{
    /// Resolve a sequence set, answering BAD for invalid message numbers.
    pub(super) fn resolve_set(
        &mut self,
        tag: &str,
        set: &SequenceSet,
        uid: bool,
    ) -> Option<Vec<usize>> {
        let selected = self.selected.as_ref().expect("selected");
        match selected.resolve(set, uid) {
            Resolution::Indexes(indexes) => Some(indexes),
            Resolution::InvalidSequence => {
                self.connection.output.status(
                    Some(tag),
                    Status::Bad,
                    None,
                    "Invalid message sequence number",
                );
                None
            }
        }
    }

    /// Answer STORE or UID STORE.
    pub(super) async fn store(
        &mut self,
        tag: &str,
        uid: bool,
        set: &SequenceSet,
        change: &FlagChange,
        silent: bool,
    ) -> std::io::Result<Next> {
        if self.selected.as_ref().expect("selected").read_only {
            self.connection
                .output
                .status(Some(tag), Status::No, None, "Mailbox is read-only");
            return Ok(Next::Continue);
        }
        let Some(indexes) = self.resolve_set(tag, set, uid) else {
            return Ok(Next::Continue);
        };
        let (uids, mut missing) = {
            let selected = self.selected.as_ref().expect("selected");
            let mut missing = false;
            let uids: Vec<Uid> = indexes
                .iter()
                .map(|index| &selected.entries[*index])
                .filter(|entry| {
                    missing |= entry.expunged;
                    !entry.expunged
                })
                .map(|entry| entry.uid)
                .collect();
            (uids, missing)
        };
        if !uids.is_empty() {
            let limit = self.limits().backend_timeout;
            let selected = self.selected.as_ref().expect("selected");
            let account = &self.account.as_ref().expect("authenticated").account;
            let updated = match bounded(limit, account.store(&selected.handle, &uids, change)).await
            {
                Ok(Ok(updated)) => updated,
                Ok(Err(error)) => return self.mailbox_failure(tag, error),
                Err(error) => return self.mailbox_failure(tag, MailboxError::Backend(error)),
            };
            missing |= updated.len() < uids.len();
            let selected = self.selected.as_mut().expect("selected");
            let mut written = Vec::new();
            for (uid, flags) in updated {
                if let Some(index) = selected.index_of_uid(uid) {
                    selected.entries[index].flags = flags;
                    written.push(index);
                }
            }
            selected.announce_new_keywords(&mut self.connection.output);
            if !silent {
                for index in written {
                    selected.write_flags_fetch(index, uid, &mut self.connection.output);
                }
            }
        }
        if missing && !uid {
            self.no(
                tag,
                ResponseCode::ExpungeIssued,
                "Some messages were removed and could not be changed",
            );
        } else {
            self.ok(tag, "STORE completed");
        }
        Ok(Next::Continue)
    }

    /// Answer EXPUNGE or UID EXPUNGE.
    pub(super) async fn expunge(
        &mut self,
        tag: &str,
        uids: Option<&SequenceSet>,
    ) -> std::io::Result<Next> {
        if self.selected.as_ref().expect("selected").read_only {
            self.connection
                .output
                .status(Some(tag), Status::No, None, "Mailbox is read-only");
            return Ok(Next::Continue);
        }
        let restriction: Option<Vec<Uid>> = match uids {
            Some(set) => {
                let Some(indexes) = self.resolve_set(tag, set, true) else {
                    return Ok(Next::Continue);
                };
                let selected = self.selected.as_ref().expect("selected");
                Some(
                    indexes
                        .iter()
                        .map(|index| selected.entries[*index].uid)
                        .collect(),
                )
            }
            None => None,
        };
        if restriction.as_ref().is_some_and(Vec::is_empty) {
            self.ok(tag, "EXPUNGE completed");
            return Ok(Next::Continue);
        }
        let limit = self.limits().backend_timeout;
        let selected = self.selected.as_ref().expect("selected");
        let account = &self.account.as_ref().expect("authenticated").account;
        let removed = match bounded(
            limit,
            account.expunge(&selected.handle, restriction.as_deref()),
        )
        .await
        {
            Ok(Ok(removed)) => removed,
            Ok(Err(error)) => return self.mailbox_failure(tag, error),
            Err(error) => return self.mailbox_failure(tag, MailboxError::Backend(error)),
        };
        let selected = self.selected.as_mut().expect("selected");
        selected.mark_expunged(&removed);
        selected.announce_expunges(&mut self.connection.output);
        self.ok(tag, "EXPUNGE completed");
        Ok(Next::Continue)
    }

    /// Answer COPY, MOVE, and their UID forms.
    pub(super) async fn copy(
        &mut self,
        tag: &str,
        uid: bool,
        set: &SequenceSet,
        destination: &str,
        remove_from_source: bool,
    ) -> std::io::Result<Next> {
        let command = if remove_from_source { "MOVE" } else { "COPY" };
        if remove_from_source && self.selected.as_ref().expect("selected").read_only {
            self.connection
                .output
                .status(Some(tag), Status::No, None, "Mailbox is read-only");
            return Ok(Next::Continue);
        }
        let Some(indexes) = self.resolve_set(tag, set, uid) else {
            return Ok(Next::Continue);
        };
        let (uids, missing) = {
            let selected = self.selected.as_ref().expect("selected");
            let missing = indexes
                .iter()
                .any(|index| selected.entries[*index].expunged);
            let uids: Vec<Uid> = indexes
                .iter()
                .map(|index| &selected.entries[*index])
                .filter(|entry| !entry.expunged)
                .map(|entry| entry.uid)
                .collect();
            (uids, missing)
        };
        if missing && !uid {
            self.no(
                tag,
                ResponseCode::ExpungeIssued,
                "Some messages were removed; nothing was copied",
            );
            return Ok(Next::Continue);
        }
        if uids.is_empty() {
            self.ok(tag, &format!("{command} completed"));
            return Ok(Next::Continue);
        }
        let limit = self.limits().backend_timeout;
        let selected = self.selected.as_ref().expect("selected");
        let account = &self.account.as_ref().expect("authenticated").account;
        let outcome = match bounded(
            limit,
            account.copy(&selected.handle, &uids, destination, remove_from_source),
        )
        .await
        {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(MailboxError::NonExistent)) => {
                self.no(
                    tag,
                    ResponseCode::TryCreate,
                    "Destination mailbox does not exist",
                );
                return Ok(Next::Continue);
            }
            Ok(Err(error)) => return self.mailbox_failure(tag, error),
            Err(error) => return self.mailbox_failure(tag, MailboxError::Backend(error)),
        };
        let copy_uid = (!outcome.destination_uids.is_empty()
            && outcome.destination_uids.len() == outcome.source_uids.len())
        .then(|| ResponseCode::CopyUid {
            uid_validity: outcome.uid_validity,
            source: format_ordered_set(&outcome.source_uids),
            destination: format_ordered_set(&outcome.destination_uids),
        });
        if remove_from_source {
            if let Some(code) = &copy_uid {
                self.connection
                    .output
                    .status(None, Status::Ok, Some(code), "Moved");
            }
            let selected = self.selected.as_mut().expect("selected");
            selected.mark_expunged(&outcome.expunged);
            selected.announce_expunges(&mut self.connection.output);
            self.ok(tag, "MOVE completed");
        } else {
            self.connection.output.status(
                Some(tag),
                Status::Ok,
                copy_uid.as_ref(),
                "COPY completed",
            );
        }
        Ok(Next::Continue)
    }
}
