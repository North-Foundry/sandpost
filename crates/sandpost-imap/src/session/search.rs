//! SEARCH execution and criterion evaluation against the selected session's message positions.
use super::{Next, Session, bounded};
use crate::{
    backend::{Account, ImapBackend, MailboxError, MessageContent, MessageMetadata, Uid},
    limits::METADATA_BATCH_SIZE,
    message::searchable::SearchableMessage,
    protocol::{
        command::SearchKey,
        datetime::CivilDate,
        flags::{Flags, SystemFlag},
        response::ResponseCode,
    },
};
use std::collections::HashMap;
use tokio::io::{AsyncRead, AsyncWrite};

/// What a SEARCH criterion can be evaluated against.
struct SearchSubject<'subject> {
    sequence: u32,
    uid: Uid,
    flags: &'subject Flags,
    recent: bool,
    metadata: Option<MessageMetadata>,
    message: Option<&'subject SearchableMessage<'subject>>,
    internal_date: Option<i64>,
    message_count: u32,
    largest_uid: Uid,
}

impl<Stream, Backend> Session<Stream, Backend>
where
    Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    Backend: ImapBackend,
{
    /// Answer SEARCH or UID SEARCH by evaluating the criteria against each message.
    pub(super) async fn search(
        &mut self,
        tag: &str,
        uid: bool,
        charset: Option<&str>,
        criteria: &SearchKey,
    ) -> std::io::Result<Next> {
        if let Some(charset) = charset
            && charset != "US-ASCII"
            && charset != "UTF-8"
        {
            self.no(
                tag,
                ResponseCode::BadCharset("(US-ASCII UTF-8)".into()),
                "Unsupported charset",
            );
            return Ok(Next::Continue);
        }
        let limit = self.limits().backend_timeout;
        let needs_content = criteria.needs_content();
        let needs_metadata = criteria.needs_metadata();
        let candidates: Vec<(usize, Uid)> = {
            let selected = self.selected.as_ref().expect("selected");
            selected
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| !entry.expunged)
                .map(|(index, entry)| (index, entry.uid))
                .collect()
        };
        let mut metadata: HashMap<Uid, MessageMetadata> = HashMap::new();
        if needs_metadata && !needs_content {
            let uids: Vec<Uid> = candidates.iter().map(|(_, uid)| *uid).collect();
            for chunk in uids.chunks(METADATA_BATCH_SIZE) {
                let selected = self.selected.as_ref().expect("selected");
                let account = &self.account.as_ref().expect("authenticated").account;
                match bounded(limit, account.metadata(&selected.handle, chunk)).await {
                    Ok(Ok(loaded)) => {
                        metadata.extend(loaded.into_iter().map(|item| (item.uid, item)));
                    }
                    Ok(Err(error)) => return self.mailbox_failure(tag, error),
                    Err(error) => return self.mailbox_failure(tag, MailboxError::Backend(error)),
                }
            }
        }
        let (message_count, largest_uid) = {
            let selected = self.selected.as_ref().expect("selected");
            (
                selected.entries.len() as u32,
                selected
                    .entries
                    .iter()
                    .rfind(|entry| !entry.expunged)
                    .map_or(0, |entry| entry.uid),
            )
        };
        let mut matches = Vec::new();
        for (index, entry_uid) in candidates {
            let content: Option<MessageContent> = if needs_content {
                let selected = self.selected.as_ref().expect("selected");
                let account = &self.account.as_ref().expect("authenticated").account;
                match bounded(limit, account.content(&selected.handle, entry_uid)).await {
                    Ok(Ok(Some(content))) => Some(content),
                    Ok(Ok(None)) => continue,
                    Ok(Err(error)) => return self.mailbox_failure(tag, error),
                    Err(error) => return self.mailbox_failure(tag, MailboxError::Backend(error)),
                }
            } else {
                None
            };
            let searchable = content
                .as_ref()
                .map(|content| SearchableMessage::new(&content.octets));
            let selected = self.selected.as_ref().expect("selected");
            let entry = &selected.entries[index];
            let item_metadata = match &content {
                Some(content) => Some(MessageMetadata {
                    uid: entry_uid,
                    size: content.octets.len() as u64,
                    internal_date: content.internal_date,
                }),
                None => metadata.get(&entry_uid).copied(),
            };
            if needs_metadata && item_metadata.is_none() {
                continue;
            }
            let subject = SearchSubject {
                sequence: index as u32 + 1,
                uid: entry_uid,
                flags: &entry.flags,
                recent: entry.recent,
                metadata: item_metadata,
                message: searchable.as_ref(),
                internal_date: item_metadata.map(|item| item.internal_date),
                message_count,
                largest_uid,
            };
            if evaluate(criteria, &subject) {
                matches.push(if uid { entry_uid } else { index as u32 + 1 });
            }
        }
        self.connection.output.raw("* SEARCH");
        for value in matches {
            self.connection.output.raw(" ").number(value);
        }
        self.connection.output.end_line();
        self.ok(tag, "SEARCH completed");
        Ok(Next::Continue)
    }
}

/// Evaluate a SEARCH key for one message (RFC 3501 section 6.4.4).
fn evaluate(key: &SearchKey, subject: &SearchSubject<'_>) -> bool {
    let header = |name: &str, needle: &str| {
        subject
            .message
            .is_some_and(|message| message.header_contains(name, needle))
    };
    let internal_day = || subject.internal_date.map(CivilDate::from_unix_seconds);
    let sent_day = || subject.message.and_then(SearchableMessage::sent_date);
    let size = || subject.metadata.map(|metadata| metadata.size);
    match key {
        SearchKey::All => true,
        SearchKey::And(keys) => keys.iter().all(|key| evaluate(key, subject)),
        SearchKey::Or(left, right) => evaluate(left, subject) || evaluate(right, subject),
        SearchKey::Not(key) => !evaluate(key, subject),
        SearchKey::Answered => subject.flags.contains(SystemFlag::Answered),
        SearchKey::Deleted => subject.flags.contains(SystemFlag::Deleted),
        SearchKey::Draft => subject.flags.contains(SystemFlag::Draft),
        SearchKey::Flagged => subject.flags.contains(SystemFlag::Flagged),
        SearchKey::Seen => subject.flags.contains(SystemFlag::Seen),
        SearchKey::Unanswered => !subject.flags.contains(SystemFlag::Answered),
        SearchKey::Undeleted => !subject.flags.contains(SystemFlag::Deleted),
        SearchKey::Undraft => !subject.flags.contains(SystemFlag::Draft),
        SearchKey::Unflagged => !subject.flags.contains(SystemFlag::Flagged),
        SearchKey::Unseen => !subject.flags.contains(SystemFlag::Seen),
        SearchKey::Keyword(keyword) => subject.flags.has_keyword(keyword),
        SearchKey::Unkeyword(keyword) => !subject.flags.has_keyword(keyword),
        SearchKey::Recent => subject.recent,
        SearchKey::New => subject.recent && !subject.flags.contains(SystemFlag::Seen),
        SearchKey::Old => !subject.recent,
        SearchKey::Larger(bound) => size().is_some_and(|size| size > u64::from(*bound)),
        SearchKey::Smaller(bound) => size().is_some_and(|size| size < u64::from(*bound)),
        SearchKey::Before(date) => internal_day().is_some_and(|day| day < *date),
        SearchKey::On(date) => internal_day().is_some_and(|day| day == *date),
        SearchKey::Since(date) => internal_day().is_some_and(|day| day >= *date),
        SearchKey::SentBefore(date) => sent_day().is_some_and(|day| day < *date),
        SearchKey::SentOn(date) => sent_day().is_some_and(|day| day == *date),
        SearchKey::SentSince(date) => sent_day().is_some_and(|day| day >= *date),
        SearchKey::Uid(set) => set.contains(subject.uid, subject.largest_uid),
        SearchKey::Sequence(set) => set.contains(subject.sequence, subject.message_count),
        SearchKey::From(needle) => header("from", needle),
        SearchKey::To(needle) => header("to", needle),
        SearchKey::Cc(needle) => header("cc", needle),
        SearchKey::Bcc(needle) => header("bcc", needle),
        SearchKey::Subject(needle) => header("subject", needle),
        SearchKey::Header(name, needle) => header(name, needle),
        SearchKey::Body(needle) => subject
            .message
            .is_some_and(|message| message.body_contains(needle)),
        SearchKey::Text(needle) => subject
            .message
            .is_some_and(|message| message.text_contains(needle)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::sequence::SequenceSet;

    const MESSAGE: &[u8] = b"From: Ann <ann@example.test>\r\nSubject: Hello\r\n\r\nBody line\r\n";

    /// SEARCH keys combine flags, sequence sets, sizes, dates, and content.
    #[test]
    fn search_evaluation() {
        let message = SearchableMessage::new(MESSAGE);
        let mut flags = Flags::new();
        flags.insert(SystemFlag::Flagged);
        flags.insert_keyword("$Work");
        let subject = SearchSubject {
            sequence: 2,
            uid: 20,
            flags: &flags,
            recent: true,
            metadata: Some(MessageMetadata {
                uid: 20,
                size: 59,
                internal_date: 1_700_000_000,
            }),
            message: Some(&message),
            internal_date: Some(1_700_000_000),
            message_count: 3,
            largest_uid: 30,
        };
        let date = |text: &str| CivilDate::parse(text).unwrap();
        let set = |text: &str| SequenceSet::parse(text).unwrap();
        for (key, expected) in [
            (SearchKey::New, true),
            (SearchKey::Old, false),
            (SearchKey::Keyword("$work".into()), true),
            (SearchKey::Unflagged, false),
            (SearchKey::Larger(58), true),
            (SearchKey::Smaller(59), false),
            (SearchKey::On(date("14-Nov-2023")), true),
            (SearchKey::Before(date("14-Nov-2023")), false),
            (SearchKey::Since(date("1-Jan-2023")), true),
            (SearchKey::SentOn(date("14-Nov-2023")), false),
            (SearchKey::Uid(set("15:*")), true),
            (SearchKey::Sequence(set("3")), false),
            (SearchKey::From("ANN".into()), true),
            (SearchKey::Subject("bye".into()), false),
            (SearchKey::Body("body LINE".into()), true),
            (SearchKey::Text("hello".into()), true),
            (
                SearchKey::Or(
                    Box::new(SearchKey::Seen),
                    Box::new(SearchKey::Not(Box::new(SearchKey::Draft))),
                ),
                true,
            ),
            (
                SearchKey::And(vec![
                    SearchKey::Flagged,
                    SearchKey::Header("x-none".into(), "".into()),
                ]),
                false,
            ),
        ] {
            assert_eq!(evaluate(&key, &subject), expected, "{key:?}");
        }
    }
}
