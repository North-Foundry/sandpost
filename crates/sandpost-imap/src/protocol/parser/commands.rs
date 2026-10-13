//! Parsing for command dispatch and mailbox, authentication, and flag commands.

use super::cursor::{Cursor, ParseResult};
use crate::protocol::{
    command::{CommandKind, StatusItem},
    datetime::parse_internal_date,
    flags::{FlagChange, FlagOperation, Flags, SystemFlag, is_valid_keyword},
    mailbox_name,
    syntax::is_base64_response,
};

impl<'input> Cursor<'input> {
    /// Read the command name and arguments that follow the tag.
    pub(super) fn command_after_tag(&mut self) -> ParseResult<CommandKind> {
        self.space()?;
        let name = self.atom()?.to_ascii_uppercase();
        let kind = match name.as_str() {
            "CAPABILITY" => CommandKind::Capability,
            "NOOP" => CommandKind::Noop,
            "LOGOUT" => CommandKind::Logout,
            "STARTTLS" => CommandKind::StartTls,
            "CHECK" => CommandKind::Check,
            "CLOSE" => CommandKind::Close,
            "UNSELECT" => CommandKind::Unselect,
            "EXPUNGE" => CommandKind::Expunge { uids: None },
            "IDLE" => CommandKind::Idle,
            "NAMESPACE" => CommandKind::Namespace,
            "AUTHENTICATE" => self.authenticate()?,
            "LOGIN" => {
                self.space()?;
                let username = self.text_astring()?;
                self.space()?;
                let password = self.text_astring()?;
                CommandKind::Login { username, password }
            }
            "SELECT" | "EXAMINE" => {
                self.space()?;
                let mailbox = self.mailbox()?;
                if self.peek() == Some(b' ') {
                    return Err("SELECT parameters are not supported".into());
                }
                CommandKind::Select {
                    mailbox,
                    read_only: name == "EXAMINE",
                }
            }
            "CREATE" | "DELETE" => {
                self.space()?;
                let mailbox = self.mailbox()?;
                if name == "CREATE" && self.peek() == Some(b' ') {
                    return Err("CREATE parameters are not supported".into());
                }
                if name == "CREATE" {
                    CommandKind::Create { mailbox }
                } else {
                    CommandKind::Delete { mailbox }
                }
            }
            "RENAME" => {
                self.space()?;
                let source = self.mailbox()?;
                self.space()?;
                let destination = self.mailbox()?;
                CommandKind::Rename {
                    source,
                    destination,
                }
            }
            "SUBSCRIBE" | "UNSUBSCRIBE" => {
                self.space()?;
                CommandKind::Subscribe {
                    mailbox: self.mailbox()?,
                    subscribe: name == "SUBSCRIBE",
                }
            }
            "LIST" | "LSUB" => {
                self.space()?;
                if self.peek() == Some(b'(') {
                    return Err("LIST-EXTENDED selection options are not supported".into());
                }
                let reference = self.mailbox_reference()?;
                self.space()?;
                let pattern = self.list_mailbox()?;
                if self.peek() == Some(b' ') {
                    return Err("LIST-EXTENDED return options are not supported".into());
                }
                CommandKind::List {
                    reference,
                    pattern,
                    subscribed_only: name == "LSUB",
                }
            }
            "STATUS" => self.status()?,
            "APPEND" => self.append()?,
            "SEARCH" => self.search(false)?,
            "FETCH" => self.fetch(false)?,
            "STORE" => self.store(false)?,
            "COPY" => self.copy(false, false)?,
            "MOVE" => self.copy(false, true)?,
            "ID" => self.id()?,
            "UID" => {
                self.space()?;
                let subcommand = self.atom()?.to_ascii_uppercase();
                match subcommand.as_str() {
                    "SEARCH" => self.search(true)?,
                    "FETCH" => self.fetch(true)?,
                    "STORE" => self.store(true)?,
                    "COPY" => self.copy(true, false)?,
                    "MOVE" => self.copy(true, true)?,
                    "EXPUNGE" => {
                        self.space()?;
                        CommandKind::Expunge {
                            uids: Some(self.sequence_set()?),
                        }
                    }
                    _ => return Err("unknown UID command".into()),
                }
            }
            _ => return Err("unknown command".into()),
        };
        self.end()?;
        Ok(kind)
    }

    /// Read a LIST reference name, which may be empty and is not INBOX-normalized.
    pub(super) fn mailbox_reference(&mut self) -> ParseResult<String> {
        let raw = self.astring()?;
        mailbox_name::decode(&raw).ok_or_else(|| "invalid reference name".to_owned())
    }

    /// Read `AUTHENTICATE mechanism [SP initial-response]`.
    pub(super) fn authenticate(&mut self) -> ParseResult<CommandKind> {
        self.space()?;
        let mechanism = self.atom()?.to_ascii_uppercase();
        let initial_response = if self.peek() == Some(b' ') {
            self.space()?;
            let response = self.take_while(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')
            });
            if response != b"=" && (response.is_empty() || !is_base64_response(response)) {
                return Err("invalid initial response".into());
            }
            Some(String::from_utf8_lossy(response).into_owned())
        } else {
            None
        };
        Ok(CommandKind::Authenticate {
            mechanism,
            initial_response,
        })
    }

    /// Read `STATUS mailbox (items)`.
    pub(super) fn status(&mut self) -> ParseResult<CommandKind> {
        self.space()?;
        let mailbox = self.mailbox()?;
        self.space()?;
        self.expect(b'(', "a status item list")?;
        let mut items = Vec::new();
        loop {
            let item = match self.atom()?.to_ascii_uppercase().as_str() {
                "MESSAGES" => StatusItem::Messages,
                "RECENT" => StatusItem::Recent,
                "UIDNEXT" => StatusItem::UidNext,
                "UIDVALIDITY" => StatusItem::UidValidity,
                "UNSEEN" => StatusItem::Unseen,
                _ => return Err("unsupported STATUS item".into()),
            };
            if !items.contains(&item) {
                items.push(item);
            }
            match self.peek() {
                Some(b' ') => self.position += 1,
                Some(b')') => {
                    self.position += 1;
                    break;
                }
                _ => return Err("expected a space or closing parenthesis".into()),
            }
        }
        Ok(CommandKind::Status { mailbox, items })
    }

    /// Read `APPEND mailbox [flag-list] [date-time] literal`, validating every argument.
    pub(super) fn append(&mut self) -> ParseResult<CommandKind> {
        self.space()?;
        let mailbox = self.mailbox()?;
        self.space()?;
        if self.peek() == Some(b'(') {
            self.flag_list(false)?;
            self.space()?;
        }
        if self.peek() == Some(b'"') {
            let date = self.quoted()?;
            std::str::from_utf8(&date)
                .ok()
                .and_then(parse_internal_date)
                .ok_or_else(|| "invalid APPEND date-time".to_owned())?;
            self.space()?;
        }
        if self.peek() != Some(b'{') {
            return Err("expected the message literal".into());
        }
        self.literal()?;
        Ok(CommandKind::Append { mailbox })
    }

    /// Read `[UID] COPY|MOVE set mailbox`.
    pub(super) fn copy(&mut self, uid: bool, remove_from_source: bool) -> ParseResult<CommandKind> {
        self.space()?;
        let set = self.sequence_set()?;
        self.space()?;
        let mailbox = self.mailbox()?;
        Ok(CommandKind::Copy {
            uid,
            set,
            mailbox,
            remove_from_source,
        })
    }

    /// Read `ID (field value ...)` or `ID NIL` (RFC 2971), validating the parameter list.
    pub(super) fn id(&mut self) -> ParseResult<CommandKind> {
        self.space()?;
        if self.peek() == Some(b'(') {
            self.position += 1;
            let mut fields: Vec<Vec<u8>> = Vec::new();
            while self.peek() != Some(b')') {
                if !fields.is_empty() {
                    self.space()?;
                }
                let field = self.string()?;
                if field.len() > 30 {
                    return Err("ID field name too long".into());
                }
                if fields
                    .iter()
                    .any(|existing| existing.as_slice().eq_ignore_ascii_case(&field))
                {
                    return Err("duplicate ID field name".into());
                }
                self.space()?;
                if self.nstring()?.is_some_and(|value| value.len() > 1024) {
                    return Err("ID field value too long".into());
                }
                fields.push(field);
                if fields.len() > 30 {
                    return Err("too many ID parameters".into());
                }
            }
            self.position += 1;
        } else if !self.atom()?.eq_ignore_ascii_case("NIL") {
            return Err("expected an ID parameter list or NIL".into());
        }
        Ok(CommandKind::Id)
    }

    /// Read a parenthesized flag list for STORE or APPEND.
    ///
    /// `\Recent` and `\*` are never storable; unknown system flags are rejected.
    pub(super) fn flag_list(&mut self, allow_bare: bool) -> ParseResult<Flags> {
        let parenthesized = self.peek() == Some(b'(');
        if parenthesized {
            self.position += 1;
        } else if !allow_bare {
            return Err("expected a flag list".into());
        }
        let mut flags = Flags::new();
        let mut first = true;
        loop {
            if parenthesized && self.peek() == Some(b')') {
                self.position += 1;
                break;
            }
            if !parenthesized && !first && self.peek() != Some(b' ') {
                break;
            }
            if !first {
                self.space()?;
            }
            first = false;
            if self.peek() == Some(b'\\') {
                self.position += 1;
                let name = format!("\\{}", self.atom()?);
                let flag = SystemFlag::from_name(&name)
                    .ok_or_else(|| format!("unsupported system flag {name}"))?;
                flags.insert(flag);
            } else {
                let keyword = self.atom()?;
                if !is_valid_keyword(keyword) {
                    return Err("invalid keyword".into());
                }
                flags.insert_keyword(keyword);
                if flags.keywords().len() > crate::protocol::flags::MAXIMUM_KEYWORDS_PER_MESSAGE {
                    return Err("too many keywords".into());
                }
            }
            if !parenthesized && self.peek() != Some(b' ') {
                break;
            }
        }
        Ok(flags)
    }

    /// Read `[UID] STORE set (+|-)FLAGS[.SILENT] flags`.
    pub(super) fn store(&mut self, uid: bool) -> ParseResult<CommandKind> {
        self.space()?;
        let set = self.sequence_set()?;
        self.space()?;
        let operation = match self.peek() {
            Some(b'+') => {
                self.position += 1;
                FlagOperation::Add
            }
            Some(b'-') => {
                self.position += 1;
                FlagOperation::Remove
            }
            _ => FlagOperation::Replace,
        };
        let item = self.take_while(|byte| byte.is_ascii_alphabetic() || byte == b'.');
        let silent = if item.eq_ignore_ascii_case(b"FLAGS") {
            false
        } else if item.eq_ignore_ascii_case(b"FLAGS.SILENT") {
            true
        } else {
            return Err("expected FLAGS or FLAGS.SILENT".into());
        };
        self.space()?;
        let flags = self.flag_list(true)?;
        Ok(CommandKind::Store {
            uid,
            set,
            change: FlagChange { operation, flags },
            silent,
        })
    }
}
