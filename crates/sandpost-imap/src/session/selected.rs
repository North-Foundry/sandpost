//! The selected mailbox as one session sees it: message sequence numbers, flags, `\Recent`, and
//! removals that have not been announced yet.
//!
//! Sequence numbers are positions in `entries`. A message the backend removed stays in place,
//! marked `expunged`, until an EXPUNGE response may be sent (RFC 3501 section 7.4.1 forbids them
//! during FETCH, STORE, and SEARCH), so the client and server never disagree about numbering.
use crate::backend::{MailboxChanges, MailboxSnapshot, MessageState, SynchronizationRequest, Uid};
use crate::protocol::{
    flags::{Flags, SystemFlag},
    response::{Output, ResponseCode, Status},
    sequence::SequenceSet,
};

/// One message in the session's view of the mailbox.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub(crate) uid: Uid,
    pub(crate) flags: Flags,
    pub(crate) recent: bool,
    /// Removed by the backend, awaiting an EXPUNGE response.
    pub(crate) expunged: bool,
}

/// A selected mailbox.
pub(crate) struct SelectedMailbox<Handle> {
    pub(crate) handle: Handle,
    pub(crate) read_only: bool,
    pub(crate) requested_read_only: bool,
    pub(crate) seen_only: bool,
    pub(crate) uid_validity: u32,
    pub(crate) uid_next: Uid,
    pub(crate) entries: Vec<Entry>,
    /// Keywords announced in the last FLAGS response.
    keywords: Vec<String>,
    cursor: u64,
}

/// Whether a resolved sequence set is usable.
pub(crate) enum Resolution {
    /// Indexes into `entries`, ascending.
    Indexes(Vec<usize>),
    /// A message sequence number beyond the mailbox size.
    InvalidSequence,
}

impl<Handle> SelectedMailbox<Handle> {
    /// Build the session state from a backend snapshot.
    pub(crate) fn new(snapshot: MailboxSnapshot<Handle>, read_only: bool) -> Self {
        let mut mailbox = Self {
            handle: snapshot.handle,
            read_only: read_only || snapshot.read_only,
            requested_read_only: read_only,
            seen_only: snapshot.seen_only,
            uid_validity: snapshot.uid_validity,
            uid_next: snapshot.uid_next,
            entries: snapshot
                .messages
                .into_iter()
                .map(|message| Entry {
                    uid: message.uid,
                    flags: message.flags,
                    recent: message.recent,
                    expunged: false,
                })
                .collect(),
            keywords: Vec::new(),
            cursor: snapshot.cursor,
        };
        mailbox.keywords = mailbox.all_keywords();
        mailbox
    }

    /// Write the untagged responses of a successful SELECT or EXAMINE (RFC 3501 section 6.3.1).
    pub(crate) fn write_selection(&self, output: &mut Output) {
        output.raw("* FLAGS ").raw(&self.flags_list()).end_line();
        let permanent = if self.read_only {
            "()".to_owned()
        } else if self.seen_only {
            "(\\Seen)".to_owned()
        } else {
            let mut list = self.flags_list();
            list.pop();
            if list.len() > 1 {
                list.push(' ');
            }
            list.push_str("\\*)");
            list
        };
        output.status(
            None,
            Status::Ok,
            Some(&ResponseCode::PermanentFlags(permanent)),
            "Flags permitted",
        );
        output
            .raw(&format!("* {} EXISTS", self.entries.len()))
            .end_line();
        output
            .raw(&format!("* {} RECENT", self.recent_count()))
            .end_line();
        if let Some(position) = self
            .entries
            .iter()
            .position(|entry| !entry.flags.contains(SystemFlag::Seen))
        {
            output.status(
                None,
                Status::Ok,
                Some(&ResponseCode::Unseen(position as u32 + 1)),
                "First unseen message",
            );
        }
        output.status(
            None,
            Status::Ok,
            Some(&ResponseCode::UidValidity(self.uid_validity)),
            "UIDs valid",
        );
        output.status(
            None,
            Status::Ok,
            Some(&ResponseCode::UidNext(self.uid_next)),
            "Predicted next UID",
        );
    }

    /// The FLAGS list: system flags then every keyword in use.
    fn flags_list(&self) -> String {
        let mut list = String::from("(\\Answered \\Flagged \\Deleted \\Seen \\Draft");
        for keyword in &self.keywords {
            list.push(' ');
            list.push_str(keyword);
        }
        list.push(')');
        list
    }

    /// Every keyword carried by a message, deduplicated ASCII-case-insensitively.
    fn all_keywords(&self) -> Vec<String> {
        let mut keywords = Flags::new();
        for entry in &self.entries {
            for keyword in entry.flags.keywords() {
                keywords.insert_keyword(keyword);
            }
        }
        keywords.keywords().to_vec()
    }

    /// Number of messages that are recent for this session.
    pub(crate) fn recent_count(&self) -> usize {
        self.entries.iter().filter(|entry| entry.recent).count()
    }

    /// Describe what the session knows for the next synchronization.
    pub(crate) fn synchronization_request(&self) -> SynchronizationRequest {
        SynchronizationRequest {
            highest_uid: self.entries.last().map_or(0, |entry| entry.uid),
            known_count: self.entries.iter().filter(|entry| !entry.expunged).count() as u32,
            cursor: self.cursor,
            claim_recent: !self.read_only,
        }
    }

    /// Apply backend changes and write the untagged responses that announce them.
    ///
    /// Removals are announced only when `allow_expunge`; otherwise they stay pending.
    pub(crate) fn apply(
        &mut self,
        request: SynchronizationRequest,
        changes: MailboxChanges,
        allow_expunge: bool,
        output: &mut Output,
    ) {
        self.read_only = self.requested_read_only || changes.read_only;
        if let Some(present) = &changes.present_uids {
            for entry in self
                .entries
                .iter_mut()
                .filter(|entry| entry.uid <= request.highest_uid && !entry.expunged)
            {
                if present.binary_search(&entry.uid).is_err() {
                    entry.expunged = true;
                }
            }
        }
        if allow_expunge {
            self.announce_expunges(output);
        }
        self.apply_flag_updates(&changes.flag_updates, output);
        if !changes.added.is_empty() {
            let recent_before = self.recent_count();
            let highest = self.entries.last().map_or(0, |entry| entry.uid);
            for message in changes.added {
                if message.uid > highest {
                    self.push(message);
                }
            }
            self.announce_new_keywords(output);
            output
                .raw(&format!("* {} EXISTS", self.entries.len()))
                .end_line();
            if self.recent_count() != recent_before {
                output
                    .raw(&format!("* {} RECENT", self.recent_count()))
                    .end_line();
            }
        }
        self.uid_next = self.uid_next.max(changes.uid_next);
        self.cursor = changes.cursor;
    }

    /// Append a newly arrived message.
    fn push(&mut self, message: MessageState) {
        self.entries.push(Entry {
            uid: message.uid,
            flags: message.flags,
            recent: message.recent,
            expunged: false,
        });
    }

    /// Update cached flags and announce those that differ with unsolicited FETCH responses.
    pub(crate) fn apply_flag_updates(&mut self, updates: &[(Uid, Flags)], output: &mut Output) {
        let mut changed = Vec::new();
        for (uid, flags) in updates {
            if let Some(index) = self.index_of_uid(*uid)
                && !self.entries[index].expunged
                && self.entries[index].flags != *flags
            {
                self.entries[index].flags = flags.clone();
                changed.push(index);
            }
        }
        if changed.is_empty() {
            return;
        }
        self.announce_new_keywords(output);
        for index in changed {
            self.write_flags_fetch(index, true, output);
        }
    }

    /// Write `* n FETCH (UID u FLAGS (...))` for one entry.
    pub(crate) fn write_flags_fetch(&self, index: usize, include_uid: bool, output: &mut Output) {
        let entry = &self.entries[index];
        output.raw(&format!("* {} FETCH (", index + 1));
        if include_uid {
            output.raw(&format!("UID {} ", entry.uid));
        }
        output
            .raw("FLAGS ")
            .flag_list(&entry.flags, entry.recent)
            .raw(")")
            .end_line();
    }

    /// Announce FLAGS and PERMANENTFLAGS again when messages carry keywords not announced yet.
    pub(crate) fn announce_new_keywords(&mut self, output: &mut Output) {
        let keywords = self.all_keywords();
        let has_new = keywords.iter().any(|keyword| {
            !self
                .keywords
                .iter()
                .any(|known| known.eq_ignore_ascii_case(keyword))
        });
        if !has_new {
            return;
        }
        let mut combined = Flags::new();
        for keyword in self.keywords.iter().chain(&keywords) {
            combined.insert_keyword(keyword);
        }
        self.keywords = combined.keywords().to_vec();
        output.raw("* FLAGS ").raw(&self.flags_list()).end_line();
        if !self.read_only && !self.seen_only {
            let mut list = self.flags_list();
            list.pop();
            list.push_str(" \\*)");
            output.status(
                None,
                Status::Ok,
                Some(&ResponseCode::PermanentFlags(list)),
                "Flags permitted",
            );
        }
    }

    /// Remove pending expunged entries and write their EXPUNGE responses, highest first so each
    /// number is valid when sent.
    pub(crate) fn announce_expunges(&mut self, output: &mut Output) {
        for index in (0..self.entries.len()).rev() {
            if self.entries[index].expunged {
                self.entries.remove(index);
                output.raw(&format!("* {} EXPUNGE", index + 1)).end_line();
            }
        }
    }

    /// Mark UIDs removed by this session's own EXPUNGE or MOVE.
    pub(crate) fn mark_expunged(&mut self, uids: &[Uid]) {
        for uid in uids {
            if let Some(index) = self.index_of_uid(*uid) {
                self.entries[index].expunged = true;
            }
        }
    }

    /// Mark every known message removed, after the mailbox itself disappeared.
    pub(crate) fn mark_all_expunged(&mut self) {
        for entry in &mut self.entries {
            entry.expunged = true;
        }
    }

    /// Find the entry index of a UID.
    pub(crate) fn index_of_uid(&self, uid: Uid) -> Option<usize> {
        self.entries
            .binary_search_by_key(&uid, |entry| entry.uid)
            .ok()
    }

    /// Resolve a sequence set of message numbers or UIDs to entry indexes.
    ///
    /// Message numbers beyond the mailbox size are invalid, except that a set using `*` on an
    /// empty mailbox (such as `1:*`) matches nothing, as clients commonly send it right after
    /// SELECT. Unknown UIDs are ignored (RFC 3501 section 6.4.8).
    pub(crate) fn resolve(&self, set: &SequenceSet, uid: bool) -> Resolution {
        if uid {
            let largest = self.entries.last().map_or(0, |entry| entry.uid);
            let mut indexes = Vec::new();
            for (first, last) in set.resolve(largest) {
                let start = self.entries.partition_point(|entry| entry.uid < first);
                let end = self.entries.partition_point(|entry| entry.uid <= last);
                indexes.extend(start..end);
            }
            return Resolution::Indexes(indexes);
        }
        let count = self.entries.len() as u32;
        if count == 0 && set.uses_largest() {
            return Resolution::Indexes(Vec::new());
        }
        if set.exceeds(count) {
            return Resolution::InvalidSequence;
        }
        let mut indexes = Vec::new();
        for (first, last) in set.resolve(count) {
            if first == 0 {
                continue;
            }
            indexes.extend((first - 1) as usize..last as usize);
        }
        Resolution::Indexes(indexes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mailbox with UIDs 10, 20, 30; message 20 is seen.
    fn mailbox() -> SelectedMailbox<()> {
        let mut seen = Flags::new();
        seen.insert(SystemFlag::Seen);
        SelectedMailbox::new(
            MailboxSnapshot {
                handle: (),
                uid_validity: 7,
                uid_next: 31,
                messages: vec![
                    MessageState {
                        uid: 10,
                        flags: Flags::new(),
                        recent: false,
                    },
                    MessageState {
                        uid: 20,
                        flags: seen,
                        recent: false,
                    },
                    MessageState {
                        uid: 30,
                        flags: Flags::new(),
                        recent: true,
                    },
                ],
                seen_only: false,
                read_only: false,
                cursor: 1,
            },
            false,
        )
    }

    /// Render output as text.
    fn text(output: &Output) -> String {
        String::from_utf8(output.as_bytes().to_vec()).unwrap()
    }

    /// SELECT announces flags, counts, first unseen, UIDVALIDITY, and UIDNEXT.
    #[test]
    fn selection_responses() {
        let mut output = Output::new();
        mailbox().write_selection(&mut output);
        assert_eq!(
            text(&output),
            "* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n\
* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft \\*)] Flags permitted\r\n\
* 3 EXISTS\r\n\
* 1 RECENT\r\n\
* OK [UNSEEN 1] First unseen message\r\n\
* OK [UIDVALIDITY 7] UIDs valid\r\n\
* OK [UIDNEXT 31] Predicted next UID\r\n"
        );
    }

    /// Removals are deferred when forbidden and announced highest-first when allowed.
    #[test]
    fn expunges_are_deferred_then_announced() {
        let mut mailbox = mailbox();
        let request = mailbox.synchronization_request();
        assert_eq!(request.known_count, 3);
        let mut output = Output::new();
        mailbox.apply(
            request,
            MailboxChanges {
                present_uids: Some(vec![20]),
                uid_next: 31,
                cursor: 2,
                ..MailboxChanges::default()
            },
            false,
            &mut output,
        );
        assert!(output.is_empty());
        assert_eq!(mailbox.entries.len(), 3);
        assert_eq!(mailbox.synchronization_request().known_count, 1);
        mailbox.announce_expunges(&mut output);
        assert_eq!(text(&output), "* 3 EXPUNGE\r\n* 1 EXPUNGE\r\n");
        assert_eq!(mailbox.entries.len(), 1);
    }

    /// New messages produce EXISTS and RECENT; new keywords re-announce FLAGS first.
    #[test]
    fn additions_and_keywords_are_announced() {
        let mut mailbox = mailbox();
        let request = mailbox.synchronization_request();
        let mut flags = Flags::new();
        flags.insert_keyword("$Junk");
        let mut output = Output::new();
        mailbox.apply(
            request,
            MailboxChanges {
                added: vec![MessageState {
                    uid: 40,
                    flags,
                    recent: true,
                }],
                flag_updates: vec![(10, {
                    let mut flagged = Flags::new();
                    flagged.insert(SystemFlag::Flagged);
                    flagged
                })],
                uid_next: 41,
                cursor: 3,
                present_uids: None,
                read_only: false,
            },
            true,
            &mut output,
        );
        assert_eq!(
            text(&output),
            "* 1 FETCH (UID 10 FLAGS (\\Flagged))\r\n\
* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft $Junk)\r\n\
* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft $Junk \\*)] Flags permitted\r\n\
* 4 EXISTS\r\n\
* 2 RECENT\r\n"
        );
        assert_eq!(mailbox.uid_next, 41);
    }

    /// Sequence numbers beyond the mailbox are invalid; UID sets ignore unknown UIDs.
    #[test]
    fn sequence_sets_resolve_to_indexes() {
        let mailbox = mailbox();
        let indexes = |text: &str, uid: bool| match mailbox
            .resolve(&SequenceSet::parse(text).unwrap(), uid)
        {
            Resolution::Indexes(indexes) => Some(indexes),
            Resolution::InvalidSequence => None,
        };
        assert_eq!(indexes("2:*", false), Some(vec![1, 2]));
        assert_eq!(indexes("4", false), None);
        assert_eq!(indexes("15:25,30", true), Some(vec![1, 2]));
        assert_eq!(indexes("100:*", true), Some(vec![2]));
        assert_eq!(indexes("31:40", true), Some(vec![]));
        let empty: SelectedMailbox<()> = SelectedMailbox::new(
            MailboxSnapshot {
                handle: (),
                uid_validity: 1,
                uid_next: 1,
                messages: vec![],
                seen_only: false,
                read_only: false,
                cursor: 0,
            },
            true,
        );
        assert!(matches!(
            empty.resolve(&SequenceSet::parse("1:*").unwrap(), false),
            Resolution::Indexes(indexes) if indexes.is_empty()
        ));
        assert!(matches!(
            empty.resolve(&SequenceSet::parse("1").unwrap(), false),
            Resolution::InvalidSequence
        ));
        assert!(matches!(
            empty.resolve(&SequenceSet::parse("*").unwrap(), false),
            Resolution::Indexes(indexes) if indexes.is_empty()
        ));
    }

    /**
     * Refresh applies live role changes without ever making an EXAMINE selection writable.
     */
    #[test]
    fn synchronization_preserves_examine_and_reloads_role_read_only() {
        let mut selected = mailbox();
        let mut output = Output::new();
        let request = selected.synchronization_request();
        selected.apply(
            request,
            MailboxChanges {
                read_only: true,
                ..Default::default()
            },
            true,
            &mut output,
        );
        assert!(selected.read_only);
        let request = selected.synchronization_request();
        selected.apply(request, MailboxChanges::default(), true, &mut output);
        assert!(!selected.read_only);
        selected.requested_read_only = true;
        let request = selected.synchronization_request();
        selected.apply(request, MailboxChanges::default(), true, &mut output);
        assert!(selected.read_only);
        assert!(!selected.synchronization_request().claim_recent);
        let mut response = Output::new();
        selected.write_selection(&mut response);
        assert!(text(&response).contains("PERMANENTFLAGS ()"));
    }
}
