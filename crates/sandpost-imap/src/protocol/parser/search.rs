//! Parsing for SEARCH keys and their bounded recursive grammar.

use super::cursor::{Cursor, ParseResult};
use crate::protocol::{
    command::{CommandKind, SearchKey},
    datetime::CivilDate,
    flags::is_valid_keyword,
};

/// Deepest nesting of parenthesized or negated SEARCH keys.
const MAXIMUM_SEARCH_DEPTH: usize = 32;
/// Most SEARCH keys in one command.
const MAXIMUM_SEARCH_KEYS: usize = 256;

impl<'input> Cursor<'input> {
    /// Read `[UID] SEARCH [CHARSET name] keys`.
    pub(super) fn search(&mut self, uid: bool) -> ParseResult<CommandKind> {
        self.space()?;
        let mut charset = None;
        let checkpoint = self.position;
        if let Ok(word) = self.atom()
            && word.eq_ignore_ascii_case("CHARSET")
        {
            self.space()?;
            charset = Some(self.text_astring()?.to_ascii_uppercase());
            self.space()?;
        } else {
            self.position = checkpoint;
        }
        if self.peek() == Some(b'R') || self.peek() == Some(b'r') {
            let checkpoint = self.position;
            if let Ok(word) = self.atom()
                && word.eq_ignore_ascii_case("RETURN")
            {
                return Err("ESEARCH return options are not supported".into());
            }
            self.position = checkpoint;
        }
        let mut budget = MAXIMUM_SEARCH_KEYS;
        let mut keys = vec![self.search_key(0, &mut budget)?];
        while self.peek() == Some(b' ') {
            self.position += 1;
            keys.push(self.search_key(0, &mut budget)?);
        }
        let criteria = if keys.len() == 1 {
            keys.pop().expect("one key")
        } else {
            SearchKey::And(keys)
        };
        Ok(CommandKind::Search {
            uid,
            charset,
            criteria,
        })
    }

    /// Read one SEARCH key with bounded depth and total count.
    pub(super) fn search_key(
        &mut self,
        depth: usize,
        budget: &mut usize,
    ) -> ParseResult<SearchKey> {
        if depth > MAXIMUM_SEARCH_DEPTH {
            return Err("search criteria are nested too deeply".into());
        }
        *budget = budget
            .checked_sub(1)
            .ok_or_else(|| "too many search criteria".to_owned())?;
        match self.peek() {
            Some(b'(') => {
                self.position += 1;
                let mut keys = vec![self.search_key(depth + 1, budget)?];
                while self.peek() == Some(b' ') {
                    self.position += 1;
                    keys.push(self.search_key(depth + 1, budget)?);
                }
                self.expect(b')', "a closing parenthesis")?;
                return Ok(if keys.len() == 1 {
                    keys.pop().expect("one key")
                } else {
                    SearchKey::And(keys)
                });
            }
            Some(byte) if byte.is_ascii_digit() || byte == b'*' => {
                return Ok(SearchKey::Sequence(self.sequence_set()?));
            }
            _ => {}
        }
        let name = self.atom()?.to_ascii_uppercase();
        let key = match name.as_str() {
            "ALL" => SearchKey::All,
            "ANSWERED" => SearchKey::Answered,
            "DELETED" => SearchKey::Deleted,
            "DRAFT" => SearchKey::Draft,
            "FLAGGED" => SearchKey::Flagged,
            "NEW" => SearchKey::New,
            "OLD" => SearchKey::Old,
            "RECENT" => SearchKey::Recent,
            "SEEN" => SearchKey::Seen,
            "UNANSWERED" => SearchKey::Unanswered,
            "UNDELETED" => SearchKey::Undeleted,
            "UNDRAFT" => SearchKey::Undraft,
            "UNFLAGGED" => SearchKey::Unflagged,
            "UNSEEN" => SearchKey::Unseen,
            "BCC" => SearchKey::Bcc(self.search_string()?),
            "BODY" => SearchKey::Body(self.search_string()?),
            "CC" => SearchKey::Cc(self.search_string()?),
            "FROM" => SearchKey::From(self.search_string()?),
            "SUBJECT" => SearchKey::Subject(self.search_string()?),
            "TEXT" => SearchKey::Text(self.search_string()?),
            "TO" => SearchKey::To(self.search_string()?),
            "KEYWORD" | "UNKEYWORD" => {
                self.space()?;
                let keyword = self.atom()?.to_owned();
                if !is_valid_keyword(&keyword) {
                    return Err("invalid keyword".into());
                }
                if name == "KEYWORD" {
                    SearchKey::Keyword(keyword)
                } else {
                    SearchKey::Unkeyword(keyword)
                }
            }
            "HEADER" => {
                let field = self.search_string()?;
                let value = self.search_string()?;
                SearchKey::Header(field, value)
            }
            "BEFORE" => SearchKey::Before(self.search_date()?),
            "ON" => SearchKey::On(self.search_date()?),
            "SINCE" => SearchKey::Since(self.search_date()?),
            "SENTBEFORE" => SearchKey::SentBefore(self.search_date()?),
            "SENTON" => SearchKey::SentOn(self.search_date()?),
            "SENTSINCE" => SearchKey::SentSince(self.search_date()?),
            "LARGER" => {
                self.space()?;
                SearchKey::Larger(self.number()?)
            }
            "SMALLER" => {
                self.space()?;
                SearchKey::Smaller(self.number()?)
            }
            "UID" => {
                self.space()?;
                SearchKey::Uid(self.sequence_set()?)
            }
            "NOT" => {
                self.space()?;
                SearchKey::Not(Box::new(self.search_key(depth + 1, budget)?))
            }
            "OR" => {
                self.space()?;
                let left = self.search_key(depth + 1, budget)?;
                self.space()?;
                let right = self.search_key(depth + 1, budget)?;
                SearchKey::Or(Box::new(left), Box::new(right))
            }
            _ => return Err(format!("unsupported search key {name}")),
        };
        Ok(key)
    }

    /// Read a space and the astring argument of a SEARCH key as text.
    pub(super) fn search_string(&mut self) -> ParseResult<String> {
        self.space()?;
        let value = self.astring()?;
        String::from_utf8(value).map_err(|_| "search string is not valid UTF-8".to_owned())
    }

    /// Read a space and a SEARCH `date`, optionally quoted.
    pub(super) fn search_date(&mut self) -> ParseResult<CivilDate> {
        self.space()?;
        let text = if self.peek() == Some(b'"') {
            self.quoted()?
        } else {
            self.take_while(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                .to_vec()
        };
        std::str::from_utf8(&text)
            .ok()
            .and_then(CivilDate::parse)
            .ok_or_else(|| "invalid date".to_owned())
    }
}
