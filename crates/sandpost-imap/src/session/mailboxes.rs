//! Mailbox commands: SELECT, EXAMINE, CREATE, DELETE, RENAME, SUBSCRIBE, UNSUBSCRIBE, LIST,
//! LSUB, STATUS (RFC 3501 section 6.3), and CLOSE (section 6.4.2).
use super::{Next, Session, bounded, selected::SelectedMailbox};
use crate::{
    backend::{Account, BackendError, ImapBackend, MailboxError},
    protocol::{
        command::StatusItem,
        mailbox_name::{self, HIERARCHY_DELIMITER},
        response::{ResponseCode, Status},
    },
};
use tokio::io::{AsyncRead, AsyncWrite};

impl<Stream, Backend> Session<Stream, Backend>
where
    Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    Backend: ImapBackend,
{
    /// Answer an APPEND: messages cannot be stored, and a missing destination gets TRYCREATE.
    pub(super) async fn refuse_append(
        &mut self,
        tag: Option<&str>,
        mailbox: Option<&str>,
    ) -> std::io::Result<Next> {
        if self.account.is_none() {
            self.connection.output.status(
                tag,
                Status::Bad,
                None,
                "Command requires authentication",
            );
            return Ok(Next::Continue);
        }
        let lookup = async {
            self.validate_account().await?;
            match mailbox {
                Some(name) => Ok(self
                    .mailbox_names()
                    .await?
                    .iter()
                    .any(|existing| existing == name)),
                None => Ok(true),
            }
        }
        .await;
        let exists = match lookup {
            Ok(exists) => exists,
            Err(BackendError::AccountRevoked) => return Ok(self.revoked()),
            Err(BackendError::Unavailable(_)) => {
                self.connection.output.status(
                    tag,
                    Status::No,
                    Some(&ResponseCode::Unavailable),
                    "Temporarily unavailable, try again later",
                );
                return Ok(Next::Continue);
            }
        };
        if exists {
            self.connection.output.status(
                tag,
                Status::No,
                Some(&ResponseCode::Cannot),
                "Messages cannot be appended: Sandpost mailboxes show captured mail only",
            );
        } else {
            self.connection.output.status(
                tag,
                Status::No,
                Some(&ResponseCode::TryCreate),
                "Mailbox does not exist",
            );
        }
        Ok(Next::Continue)
    }

    /// The names of every mailbox visible to the account.
    pub(super) async fn mailbox_names(&mut self) -> Result<Vec<String>, BackendError> {
        let limit = self.limits().backend_timeout;
        let listings = bounded(limit, self.account().mailboxes()).await??;
        Ok(listings
            .into_iter()
            .filter(|listing| listing.selectable)
            .map(|listing| listing.name)
            .collect())
    }

    /// Select or examine a mailbox. Any selected mailbox is closed first without expunging,
    /// and a failed selection leaves the session authenticated (RFC 3501 section 6.3.1).
    pub(super) async fn select(
        &mut self,
        tag: &str,
        name: &str,
        read_only: bool,
    ) -> std::io::Result<Next> {
        self.selected = None;
        let limit = self.limits().backend_timeout;
        let snapshot = match bounded(limit, self.account().open(name, read_only)).await {
            Ok(Ok(snapshot)) => snapshot,
            Ok(Err(error)) => return self.mailbox_failure(tag, error),
            Err(error) => return self.mailbox_failure(tag, MailboxError::Backend(error)),
        };
        let selected = SelectedMailbox::new(snapshot, read_only);
        selected.write_selection(&mut self.connection.output);
        self.selected = Some(selected);
        let (code, command) = if read_only {
            (ResponseCode::ReadOnly, "EXAMINE")
        } else {
            (ResponseCode::ReadWrite, "SELECT")
        };
        self.connection.output.status(
            Some(tag),
            Status::Ok,
            Some(&code),
            &format!("{command} completed"),
        );
        Ok(Next::Continue)
    }

    /// Refuse to create a mailbox that exists; otherwise let the backend decide.
    pub(super) async fn create(&mut self, tag: &str, name: &str) -> std::io::Result<Next> {
        let names = match self.mailbox_names().await {
            Ok(names) => names,
            Err(error) => return self.mailbox_failure(tag, MailboxError::Backend(error)),
        };
        if mailbox_name::is_inbox(name) || names.iter().any(|existing| existing == name) {
            self.connection.output.status(
                Some(tag),
                Status::No,
                Some(&ResponseCode::AlreadyExists),
                "Mailbox already exists",
            );
            return Ok(Next::Continue);
        }
        let limit = self.limits().backend_timeout;
        let result = bounded(limit, self.account().create(name)).await;
        self.complete_management(tag, result, "CREATE completed")
    }

    /// Refuse to delete INBOX or a missing mailbox; otherwise let the backend decide.
    pub(super) async fn delete(&mut self, tag: &str, name: &str) -> std::io::Result<Next> {
        if mailbox_name::is_inbox(name) {
            self.no(tag, ResponseCode::Cannot, "INBOX cannot be deleted");
            return Ok(Next::Continue);
        }
        if let Err(failure) = self.require_existing(tag, name).await {
            return failure;
        }
        let limit = self.limits().backend_timeout;
        let result = bounded(limit, self.account().delete(name)).await;
        self.complete_management(tag, result, "DELETE completed")
    }

    /// Refuse to rename a missing mailbox; otherwise let the backend decide.
    pub(super) async fn rename(
        &mut self,
        tag: &str,
        source: &str,
        destination: &str,
    ) -> std::io::Result<Next> {
        if let Err(failure) = self.require_existing(tag, source).await {
            return failure;
        }
        let limit = self.limits().backend_timeout;
        let result = bounded(limit, self.account().rename(source, destination)).await;
        self.complete_management(tag, result, "RENAME completed")
    }

    /// Answer NONEXISTENT unless a mailbox with this name exists.
    async fn require_existing(
        &mut self,
        tag: &str,
        name: &str,
    ) -> Result<(), std::io::Result<Next>> {
        match self.mailbox_names().await {
            Ok(names) if names.iter().any(|existing| existing == name) => Ok(()),
            Ok(_) => Err(self.mailbox_failure(tag, MailboxError::NonExistent)),
            Err(error) => Err(self.mailbox_failure(tag, MailboxError::Backend(error))),
        }
    }

    /// Answer a mailbox management operation.
    fn complete_management(
        &mut self,
        tag: &str,
        result: Result<Result<(), MailboxError>, BackendError>,
        success: &str,
    ) -> std::io::Result<Next> {
        match result {
            Ok(Ok(())) => {
                self.ok(tag, success);
                Ok(Next::Continue)
            }
            Ok(Err(error)) => self.mailbox_failure(tag, error),
            Err(error) => self.mailbox_failure(tag, MailboxError::Backend(error)),
        }
    }

    /// Subscribe to or unsubscribe from a mailbox name.
    pub(super) async fn subscribe(
        &mut self,
        tag: &str,
        name: &str,
        subscribe: bool,
    ) -> std::io::Result<Next> {
        let limit = self.limits().backend_timeout;
        let result = bounded(limit, self.account().set_subscription(name, subscribe)).await;
        let success = if subscribe {
            "SUBSCRIBE completed"
        } else {
            "UNSUBSCRIBE completed"
        };
        self.complete_management(tag, result, success)
    }

    /// Answer LIST or LSUB with matching mailboxes (RFC 3501 sections 6.3.8 and 6.3.9).
    ///
    /// The namespace is flat: every mailbox is a top-level name without children. An empty
    /// pattern returns the hierarchy delimiter.
    pub(super) async fn list(
        &mut self,
        tag: &str,
        reference: &str,
        pattern: &str,
        subscribed_only: bool,
    ) -> std::io::Result<Next> {
        let command = if subscribed_only { "LSUB" } else { "LIST" };
        if pattern.is_empty() && !subscribed_only {
            self.connection
                .output
                .raw(&format!(
                    "* LIST (\\Noselect) \"{HIERARCHY_DELIMITER}\" \"\""
                ))
                .end_line();
            self.ok(tag, "LIST completed");
            return Ok(Next::Continue);
        }
        let limit = self.limits().backend_timeout;
        let listings = match bounded(limit, self.account().mailboxes()).await {
            Ok(Ok(listings)) => listings,
            Ok(Err(error)) | Err(error) => {
                return self.mailbox_failure(tag, MailboxError::Backend(error));
            }
        };
        let full_pattern = format!("{reference}{pattern}");
        for listing in listings {
            if !subscribed_only && !listing.selectable {
                continue;
            }
            if subscribed_only && !listing.subscribed {
                continue;
            }
            if !mailbox_name::matches_pattern(&listing.name, &full_pattern) {
                continue;
            }
            let mut attributes = if listing.selectable {
                vec!["\\HasNoChildren"]
            } else {
                vec!["\\Noselect"]
            };
            if let Some(special_use) = listing.special_use {
                attributes.push(special_use.attribute());
            }
            let encoded = mailbox_name::encode(&listing.name);
            self.connection.output.raw(&format!(
                "* {command} ({}) \"{HIERARCHY_DELIMITER}\" ",
                attributes.join(" ")
            ));
            self.connection
                .output
                .astring(encoded.as_bytes())
                .end_line();
        }
        self.ok(tag, &format!("{command} completed"));
        Ok(Next::Continue)
    }

    /// Answer STATUS with the requested items.
    pub(super) async fn status(
        &mut self,
        tag: &str,
        name: &str,
        items: &[StatusItem],
    ) -> std::io::Result<Next> {
        let limit = self.limits().backend_timeout;
        let status = match bounded(limit, self.account().status(name)).await {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => return self.mailbox_failure(tag, error),
            Err(error) => return self.mailbox_failure(tag, MailboxError::Backend(error)),
        };
        let values: Vec<String> = items
            .iter()
            .map(|item| {
                let value = match item {
                    StatusItem::Messages => status.messages,
                    StatusItem::Recent => status.recent,
                    StatusItem::UidNext => status.uid_next,
                    StatusItem::UidValidity => status.uid_validity,
                    StatusItem::Unseen => status.unseen,
                };
                format!("{} {value}", item.name())
            })
            .collect();
        self.connection.output.raw("* STATUS ");
        self.connection
            .output
            .astring(mailbox_name::encode(name).as_bytes());
        self.connection
            .output
            .raw(&format!(" ({})", values.join(" ")))
            .end_line();
        self.ok(tag, "STATUS completed");
        Ok(Next::Continue)
    }

    /// Expunge silently unless read-only, then return to the authenticated state.
    pub(super) async fn close(&mut self, tag: &str) -> std::io::Result<Next> {
        let selected = self.selected.take().expect("selected");
        if !selected.read_only {
            let limit = self.limits().backend_timeout;
            let result = bounded(limit, self.account().expunge(&selected.handle, None)).await;
            match result {
                Ok(Ok(_)) | Ok(Err(MailboxError::NonExistent)) => {}
                Ok(Err(error)) => return self.mailbox_failure(tag, error),
                Err(error) => return self.mailbox_failure(tag, MailboxError::Backend(error)),
            }
        }
        self.ok(tag, "CLOSE completed");
        Ok(Next::Continue)
    }
}
