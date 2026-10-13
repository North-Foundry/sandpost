//! FETCH execution and rendering, including implicit Seen updates and streamed message octets.
use super::{Next, Session, bounded};
use crate::{
    backend::{Account, ImapBackend, MailboxError, MessageContent, MessageMetadata, Uid},
    limits::{METADATA_BATCH_SIZE, OUTPUT_FLUSH_THRESHOLD},
    message::{
        body_structure::write_body_structure,
        envelope::write_envelope,
        structure::{Entity, parse_message, section_bytes},
    },
    protocol::{
        command::{FetchAttribute, Section, SectionText},
        datetime::format_internal_date,
        flags::{FlagChange, FlagOperation, Flags, SystemFlag},
        response::{Output, ResponseCode},
        sequence::SequenceSet,
    },
};
use std::collections::HashMap;
use tokio::io::{AsyncRead, AsyncWrite};

impl<Stream, Backend> Session<Stream, Backend>
where
    Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    Backend: ImapBackend,
{
    /// Answer FETCH or UID FETCH, streaming one message at a time.
    pub(super) async fn fetch(
        &mut self,
        tag: &str,
        uid: bool,
        set: &SequenceSet,
        attributes: &[FetchAttribute],
    ) -> std::io::Result<Next> {
        let Some(indexes) = self.resolve_set(tag, set, uid) else {
            return Ok(Next::Continue);
        };
        let mut attributes = attributes.to_vec();
        if uid && !attributes.contains(&FetchAttribute::Uid) {
            attributes.insert(0, FetchAttribute::Uid);
        }
        let needs_content = attributes.iter().any(FetchAttribute::needs_content);
        let needs_metadata = !needs_content
            && attributes.iter().any(|attribute| {
                matches!(
                    attribute,
                    FetchAttribute::InternalDate | FetchAttribute::Rfc822Size
                )
            });
        let limit = self.limits().backend_timeout;
        let read_only = self.selected.as_ref().expect("selected").read_only;
        let sets_seen = !read_only && attributes.iter().any(FetchAttribute::sets_seen);
        let mut metadata: HashMap<Uid, MessageMetadata> = HashMap::new();
        if needs_metadata {
            let uids: Vec<Uid> = {
                let selected = self.selected.as_ref().expect("selected");
                indexes
                    .iter()
                    .map(|index| &selected.entries[*index])
                    .filter(|entry| !entry.expunged)
                    .map(|entry| entry.uid)
                    .collect()
            };
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
        let mut missing = false;
        for index in indexes {
            let mut entry = self.selected.as_ref().expect("selected").entries[index].clone();
            if entry.expunged {
                missing = true;
                continue;
            }
            let content = if needs_content {
                let selected = self.selected.as_ref().expect("selected");
                let account = &self.account.as_ref().expect("authenticated").account;
                match bounded(limit, account.content(&selected.handle, entry.uid)).await {
                    Ok(Ok(Some(content))) => Some(content),
                    Ok(Ok(None)) => {
                        missing = true;
                        continue;
                    }
                    Ok(Err(error)) => return self.mailbox_failure(tag, error),
                    Err(error) => return self.mailbox_failure(tag, MailboxError::Backend(error)),
                }
            } else {
                None
            };
            // IMAP4rev1 literals exclude NUL (RFC 3501 section 9); keep captured bytes intact.
            if content
                .as_ref()
                .is_some_and(|content| content.octets.contains(&0))
            {
                self.no(
                    tag,
                    ResponseCode::Cannot,
                    "Message contains NUL octets unavailable through IMAP4rev1",
                );
                return Ok(Next::Continue);
            }
            let mut flags_changed = false;
            if sets_seen && !entry.flags.contains(SystemFlag::Seen) {
                let mut seen = Flags::new();
                seen.insert(SystemFlag::Seen);
                let change = FlagChange {
                    operation: FlagOperation::Add,
                    flags: seen,
                };
                let selected = self.selected.as_ref().expect("selected");
                let account = &self.account.as_ref().expect("authenticated").account;
                let updated = match bounded(
                    limit,
                    account.store(&selected.handle, &[entry.uid], &change),
                )
                .await
                {
                    Ok(Ok(updated)) => updated,
                    Ok(Err(error)) => return self.mailbox_failure(tag, error),
                    Err(error) => return self.mailbox_failure(tag, MailboxError::Backend(error)),
                };
                let Some((_, flags)) = updated.into_iter().find(|(uid, _)| *uid == entry.uid)
                else {
                    missing = true;
                    continue;
                };
                self.selected.as_mut().expect("selected").entries[index].flags = flags.clone();
                entry.flags = flags;
                flags_changed = true;
            }
            let item_metadata = match (&content, needs_metadata) {
                (Some(content), _) => Some(MessageMetadata {
                    uid: entry.uid,
                    size: content.octets.len() as u64,
                    internal_date: content.internal_date,
                }),
                (None, true) => match metadata.get(&entry.uid) {
                    Some(item) => Some(*item),
                    None => {
                        missing = true;
                        continue;
                    }
                },
                (None, false) => None,
            };
            write_fetch_response(
                &mut self.connection.output,
                index as u32 + 1,
                &entry.flags,
                entry.recent,
                entry.uid,
                &attributes,
                content.as_ref(),
                item_metadata,
                flags_changed,
            );
            if self.connection.output.len() >= OUTPUT_FLUSH_THRESHOLD {
                self.connection.flush().await?;
            }
        }
        if missing && !uid {
            self.no(
                tag,
                ResponseCode::ExpungeIssued,
                "Some messages were removed and could not be fetched",
            );
        } else {
            self.ok(tag, "FETCH completed");
        }
        Ok(Next::Continue)
    }
}

/// Write one FETCH response with the requested items in order.
#[allow(clippy::too_many_arguments)]
fn write_fetch_response(
    output: &mut Output,
    sequence: u32,
    flags: &Flags,
    recent: bool,
    uid: Uid,
    attributes: &[FetchAttribute],
    content: Option<&MessageContent>,
    metadata: Option<MessageMetadata>,
    flags_changed: bool,
) {
    let needs_structure = attributes.iter().any(|attribute| {
        !matches!(
            attribute,
            FetchAttribute::Uid
                | FetchAttribute::Flags
                | FetchAttribute::InternalDate
                | FetchAttribute::Rfc822Size
                | FetchAttribute::Rfc822
        )
    });
    let tree: Option<Entity> = content
        .filter(|_| needs_structure)
        .map(|content| parse_message(&content.octets));
    let root = || tree.as_ref().expect("structure parsed with the content");
    output.raw(&format!("* {sequence} FETCH ("));
    let mut first = true;
    let mut separator = |output: &mut Output| {
        if !std::mem::take(&mut first) {
            output.raw(" ");
        }
    };
    let mut wrote_flags = false;
    for attribute in attributes {
        separator(output);
        match attribute {
            FetchAttribute::Uid => {
                output.raw(&format!("UID {uid}"));
            }
            FetchAttribute::Flags => {
                output.raw("FLAGS ").flag_list(flags, recent);
                wrote_flags = true;
            }
            FetchAttribute::InternalDate => {
                let date = metadata.map_or(0, |metadata| metadata.internal_date);
                output.raw("INTERNALDATE ").raw(&format_internal_date(date));
            }
            FetchAttribute::Rfc822Size => {
                output.raw(&format!(
                    "RFC822.SIZE {}",
                    metadata.map_or(0, |metadata| metadata.size)
                ));
            }
            FetchAttribute::Envelope => {
                let octets = &content.expect("content loaded").octets;
                output.raw("ENVELOPE ");
                write_envelope(output, octets, &root().fields);
            }
            FetchAttribute::BodyStructure | FetchAttribute::Body => {
                let octets = &content.expect("content loaded").octets;
                let extensible = matches!(attribute, FetchAttribute::BodyStructure);
                output.raw(if extensible {
                    "BODYSTRUCTURE "
                } else {
                    "BODY "
                });
                write_body_structure(output, octets, root(), extensible);
            }
            FetchAttribute::Rfc822 => {
                let octets = &content.expect("content loaded").octets;
                output.raw("RFC822 ").literal(octets);
            }
            FetchAttribute::Rfc822Header => {
                let octets = &content.expect("content loaded").octets;
                let header = root().header.clone();
                output.raw("RFC822.HEADER ").literal(&octets[header]);
            }
            FetchAttribute::Rfc822Text => {
                let octets = &content.expect("content loaded").octets;
                let body = root().body.clone();
                output.raw("RFC822.TEXT ").literal(&octets[body]);
            }
            FetchAttribute::Section {
                section, partial, ..
            } => {
                let octets = &content.expect("content loaded").octets;
                output.raw("BODY[");
                write_section_specification(output, section);
                output.raw("]");
                let bytes = section_bytes(octets, root(), section);
                match (bytes, partial) {
                    (None, _) => {
                        output.raw(" NIL");
                    }
                    (Some(bytes), None) => {
                        output.raw(" ").literal(&bytes);
                    }
                    (Some(bytes), Some((origin, length))) => {
                        let start = (*origin as usize).min(bytes.len());
                        let end = start.saturating_add(*length as usize).min(bytes.len());
                        output
                            .raw(&format!("<{origin}> "))
                            .literal(&bytes[start..end]);
                    }
                }
            }
        }
    }
    if flags_changed && !wrote_flags {
        separator(output);
        output.raw("FLAGS ").flag_list(flags, recent);
    }
    output.raw(")").end_line();
}

/// Echo a section specification as the client requested it.
fn write_section_specification(output: &mut Output, section: &Section) {
    let part = section
        .part
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".");
    output.raw(&part);
    let Some(text) = &section.text else {
        return;
    };
    if !part.is_empty() {
        output.raw(".");
    }
    match text {
        SectionText::Header => {
            output.raw("HEADER");
        }
        SectionText::Text => {
            output.raw("TEXT");
        }
        SectionText::Mime => {
            output.raw("MIME");
        }
        SectionText::HeaderFields { not, fields } => {
            output.raw(if *not {
                "HEADER.FIELDS.NOT ("
            } else {
                "HEADER.FIELDS ("
            });
            for (index, field) in fields.iter().enumerate() {
                if index > 0 {
                    output.raw(" ");
                }
                output.astring(field.as_bytes());
            }
            output.raw(")");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const MESSAGE: &[u8] = b"From: Ann <ann@example.test>\r\nSubject: Hello\r\n\r\nBody line\r\n";

    /// Render one FETCH response with content loaded.
    fn fetch(attributes: &[FetchAttribute], flags_changed: bool) -> String {
        let mut output = Output::new();
        let content = MessageContent {
            octets: Arc::from(MESSAGE),
            internal_date: 1_700_000_000,
        };
        let mut flags = Flags::new();
        flags.insert(SystemFlag::Seen);
        write_fetch_response(
            &mut output,
            3,
            &flags,
            true,
            42,
            attributes,
            Some(&content),
            Some(MessageMetadata {
                uid: 42,
                size: MESSAGE.len() as u64,
                internal_date: 1_700_000_000,
            }),
            flags_changed,
        );
        String::from_utf8(output.as_bytes().to_vec()).unwrap()
    }

    /// Basic items are written in request order with RFC formats.
    #[test]
    fn fetch_items_follow_request_order() {
        assert_eq!(
            fetch(
                &[
                    FetchAttribute::Uid,
                    FetchAttribute::Flags,
                    FetchAttribute::Rfc822Size,
                    FetchAttribute::InternalDate
                ],
                false
            ),
            "* 3 FETCH (UID 42 FLAGS (\\Seen \\Recent) RFC822.SIZE 59 INTERNALDATE \"14-Nov-2023 22:13:20 +0000\")\r\n"
        );
    }

    /// Sections echo their specification; partial fetches report the origin only.
    #[test]
    fn sections_and_partials() {
        let response = fetch(
            &[
                FetchAttribute::Section {
                    peek: true,
                    section: Section {
                        part: vec![],
                        text: Some(SectionText::HeaderFields {
                            not: false,
                            fields: vec!["Subject".into()],
                        }),
                    },
                    partial: None,
                },
                FetchAttribute::Section {
                    peek: false,
                    section: Section {
                        part: vec![],
                        text: None,
                    },
                    partial: Some((50, 100)),
                },
                FetchAttribute::Section {
                    peek: false,
                    section: Section {
                        part: vec![2],
                        text: None,
                    },
                    partial: None,
                },
            ],
            true,
        );
        assert_eq!(
            response,
            "* 3 FETCH (BODY[HEADER.FIELDS (Subject)] {18}\r\nSubject: Hello\r\n\r\n BODY[]<50> {9}\r\ndy line\r\n BODY[2] NIL FLAGS (\\Seen \\Recent))\r\n"
        );
    }
}
