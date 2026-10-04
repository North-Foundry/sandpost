//! A small, infrastructure-independent query language for [`MessageFacts`].
use sandpost_core::MessageFacts;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

const MAX_SOURCE: usize = 16 * 1024;
const MAX_TOKENS: usize = 4096;
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    EnvelopeFromAddress,
    EnvelopeFromDomain,
    EnvelopeToAddress,
    EnvelopeToDomain,
    FromAddress,
    FromDomain,
    ToAddress,
    ToDomain,
    CcAddress,
    CcDomain,
    Subject,
    Text,
    Html,
    MessageId,
    ReceivedAt,
    Size,
    AttachmentCount,
    HasAttachments,
    Header(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Operator {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Contains,
    StartsWith,
    EndsWith,
    Matches,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Value {
    String(String),
    Number(i64),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Predicate {
    pub field: Field,
    pub op: Operator,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Expr {
    True,
    False,
    Predicate(Predicate),
    Not(Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledQuery {
    pub expression: Expr,
    fingerprint: String,
}

impl CompiledQuery {
    /// SHA-256 of the canonical expression's versioned v1 encoding.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub fn evaluate(&self, facts: &MessageFacts) -> bool {
        self.expression.evaluate(facts)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message} at byte {position}")]
pub struct QueryError {
    pub position: usize,
    pub message: String,
}

impl QueryError {
    fn at(position: usize, message: impl Into<String>) -> Self {
        Self {
            position,
            message: message.into(),
        }
    }
}

pub fn compile(source: &str) -> Result<CompiledQuery, QueryError> {
    if source.len() > MAX_SOURCE {
        return Err(QueryError::at(
            MAX_SOURCE,
            "query exceeds maximum source length",
        ));
    }
    if source.trim().is_empty() {
        return Ok(compiled(Expr::True));
    }
    let tokens = lex(source)?;
    let mut parser = Parser {
        tokens,
        cursor: 0,
        depth: 0,
    };
    let expression = parser.parse_or()?.canonicalize();
    if let Some(token) = parser.peek() {
        return Err(QueryError::at(token.start, "unexpected token"));
    }
    Ok(compiled(expression))
}

fn compiled(expression: Expr) -> CompiledQuery {
    let mut hasher = Sha256::new();
    hasher.update(b"sandpost-query\0v1\0");
    fingerprint_expr(&mut hasher, &expression);
    let digest = hasher.finalize();
    CompiledQuery {
        expression,
        fingerprint: digest.iter().map(|b| format!("{b:02x}")).collect(),
    }
}

fn fingerprint_expr(hasher: &mut Sha256, expr: &Expr) {
    match expr {
        Expr::True => hasher.update([0]),
        Expr::False => hasher.update([1]),
        Expr::Predicate(p) => {
            hasher.update([2]);
            fingerprint_field(hasher, &p.field);
            hasher.update([operator_tag(p.op)]);
            match &p.value {
                Value::String(s) => {
                    hasher.update([0]);
                    fingerprint_string(hasher, s);
                }
                Value::Number(n) => {
                    hasher.update([1]);
                    hasher.update(n.to_be_bytes());
                }
                Value::Bool(b) => hasher.update([2, u8::from(*b)]),
            }
        }
        Expr::Not(e) => {
            hasher.update([3]);
            fingerprint_expr(hasher, e);
        }
        Expr::And(xs) | Expr::Or(xs) => {
            hasher.update([if matches!(expr, Expr::And(_)) { 4 } else { 5 }]);
            hasher.update((xs.len() as u64).to_be_bytes());
            for x in xs {
                fingerprint_expr(hasher, x);
            }
        }
    }
}

fn operator_tag(op: Operator) -> u8 {
    match op {
        Operator::Eq => 0,
        Operator::Ne => 1,
        Operator::Gt => 2,
        Operator::Ge => 3,
        Operator::Lt => 4,
        Operator::Le => 5,
        Operator::Contains => 6,
        Operator::StartsWith => 7,
        Operator::EndsWith => 8,
        Operator::Matches => 9,
    }
}

fn fingerprint_string(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn fingerprint_field(hasher: &mut Sha256, field: &Field) {
    let tag = match field {
        Field::EnvelopeFromAddress => 0,
        Field::EnvelopeFromDomain => 1,
        Field::EnvelopeToAddress => 2,
        Field::EnvelopeToDomain => 3,
        Field::FromAddress => 4,
        Field::FromDomain => 5,
        Field::ToAddress => 6,
        Field::ToDomain => 7,
        Field::CcAddress => 8,
        Field::CcDomain => 9,
        Field::Subject => 10,
        Field::Text => 11,
        Field::Html => 12,
        Field::MessageId => 13,
        Field::ReceivedAt => 14,
        Field::Size => 15,
        Field::AttachmentCount => 16,
        Field::HasAttachments => 17,
        Field::Header(_) => 18,
    };
    hasher.update([tag]);
    if let Field::Header(name) = field {
        fingerprint_string(hasher, name);
    }
}

impl Expr {
    pub fn evaluate(&self, facts: &MessageFacts) -> bool {
        match self {
            Self::True => true,
            Self::False => false,
            Self::Predicate(p) => p.evaluate(facts),
            Self::Not(e) => !e.evaluate(facts),
            Self::And(xs) => xs.iter().all(|e| e.evaluate(facts)),
            Self::Or(xs) => xs.iter().any(|e| e.evaluate(facts)),
        }
    }

    pub fn canonicalize(self) -> Self {
        match self {
            Self::Not(e) => match e.canonicalize() {
                Self::True => Self::False,
                Self::False => Self::True,
                Self::Not(inner) => *inner,
                other => Self::Not(Box::new(other)),
            },
            Self::And(xs) => normalize_many(xs, true),
            Self::Or(xs) => normalize_many(xs, false),
            other => other,
        }
    }
}

fn normalize_many(items: Vec<Expr>, and: bool) -> Expr {
    let mut flat = Vec::new();
    for item in items.into_iter().map(Expr::canonicalize) {
        match (and, item) {
            (true, Expr::False) | (false, Expr::True) => {
                return if and { Expr::False } else { Expr::True };
            }
            (true, Expr::True) | (false, Expr::False) => {}
            (true, Expr::And(xs)) | (false, Expr::Or(xs)) => flat.extend(xs),
            (_, item) => flat.push(item),
        }
    }
    flat.sort();
    flat.dedup();
    if flat
        .iter()
        .any(|e| flat.binary_search(&negate(e.clone())).is_ok())
    {
        return if and { Expr::False } else { Expr::True };
    }
    match flat.len() {
        0 => {
            if and {
                Expr::True
            } else {
                Expr::False
            }
        }
        1 => flat.pop().unwrap(),
        _ => {
            if and {
                Expr::And(flat)
            } else {
                Expr::Or(flat)
            }
        }
    }
}

fn negate(expr: Expr) -> Expr {
    match expr {
        Expr::Not(inner) => *inner,
        other => Expr::Not(Box::new(other)),
    }
}

impl Predicate {
    /// Cost hint for candidate planning; exact indexed fields are cheaper than scans.
    pub fn cost(&self) -> u8 {
        match self.field {
            Field::MessageId
            | Field::ReceivedAt
            | Field::Size
            | Field::AttachmentCount
            | Field::HasAttachments => 1,
            Field::EnvelopeFromAddress
            | Field::EnvelopeFromDomain
            | Field::FromAddress
            | Field::FromDomain => 2,
            Field::Header(_) => 4,
            _ => 6,
        }
    }

    /// Returns all typed values for this field. List fields intentionally retain all entries.
    pub fn field_values(&self, facts: &MessageFacts) -> Vec<Value> {
        self.field.values(facts)
    }

    /// ANY semantics apply to every field. In particular, `!=` is any value unequal to the RHS.
    pub fn evaluate(&self, facts: &MessageFacts) -> bool {
        self.field.any_value(facts, |actual| {
            compare_ref(actual, &self.value, self.op, &self.field)
        })
    }
}

impl Field {
    pub fn values(&self, facts: &MessageFacts) -> Vec<Value> {
        let strings = |values: Vec<String>| values.into_iter().map(Value::String).collect();
        let mailboxes = |boxes: &[sandpost_core::Mailbox], domain: bool| {
            boxes
                .iter()
                .map(|m| {
                    Value::String(if domain {
                        m.domain.clone()
                    } else {
                        m.address.clone()
                    })
                })
                .collect()
        };
        match self {
            Self::EnvelopeFromAddress => facts
                .envelope_from
                .iter()
                .map(|m| Value::String(m.address.clone()))
                .collect(),
            Self::EnvelopeFromDomain => facts
                .envelope_from
                .iter()
                .map(|m| Value::String(m.domain.clone()))
                .collect(),
            Self::EnvelopeToAddress => mailboxes(&facts.envelope_to, false),
            Self::EnvelopeToDomain => mailboxes(&facts.envelope_to, true),
            Self::FromAddress => mailboxes(&facts.from, false),
            Self::FromDomain => mailboxes(&facts.from, true),
            Self::ToAddress => mailboxes(&facts.to, false),
            Self::ToDomain => mailboxes(&facts.to, true),
            Self::CcAddress => mailboxes(&facts.cc, false),
            Self::CcDomain => mailboxes(&facts.cc, true),
            Self::Subject => vec![Value::String(facts.subject.clone())],
            Self::Text => vec![Value::String(facts.text.clone())],
            Self::Html => vec![Value::String(facts.html.clone())],
            Self::MessageId => strings(facts.message_id.iter().cloned().collect()),
            Self::ReceivedAt => vec![Value::Number(facts.received_at)],
            Self::Size => vec![Value::Number(i64::try_from(facts.size).unwrap_or(i64::MAX))],
            Self::AttachmentCount => vec![Value::Number(
                i64::try_from(facts.attachment_count).unwrap_or(i64::MAX),
            )],
            Self::HasAttachments => vec![Value::Bool(facts.attachment_count > 0)],
            Self::Header(name) => strings(facts.headers.get(name).cloned().unwrap_or_default()),
        }
    }

    fn any_value(
        &self,
        facts: &MessageFacts,
        mut predicate: impl FnMut(&ValueRef<'_>) -> bool,
    ) -> bool {
        let mailboxes = |boxes: &[sandpost_core::Mailbox],
                         domain: bool,
                         f: &mut dyn FnMut(&ValueRef<'_>) -> bool| {
            boxes.iter().any(|m| {
                f(&if domain {
                    ValueRef::String(&m.domain)
                } else {
                    ValueRef::String(&m.address)
                })
            })
        };
        match self {
            Self::EnvelopeFromAddress => facts
                .envelope_from
                .iter()
                .any(|m| predicate(&ValueRef::String(&m.address))),
            Self::EnvelopeFromDomain => facts
                .envelope_from
                .iter()
                .any(|m| predicate(&ValueRef::String(&m.domain))),
            Self::EnvelopeToAddress => mailboxes(&facts.envelope_to, false, &mut predicate),
            Self::EnvelopeToDomain => mailboxes(&facts.envelope_to, true, &mut predicate),
            Self::FromAddress => mailboxes(&facts.from, false, &mut predicate),
            Self::FromDomain => mailboxes(&facts.from, true, &mut predicate),
            Self::ToAddress => mailboxes(&facts.to, false, &mut predicate),
            Self::ToDomain => mailboxes(&facts.to, true, &mut predicate),
            Self::CcAddress => mailboxes(&facts.cc, false, &mut predicate),
            Self::CcDomain => mailboxes(&facts.cc, true, &mut predicate),
            Self::Subject => predicate(&ValueRef::String(&facts.subject)),
            Self::Text => predicate(&ValueRef::String(&facts.text)),
            Self::Html => predicate(&ValueRef::String(&facts.html)),
            Self::MessageId => facts
                .message_id
                .as_deref()
                .is_some_and(|s| predicate(&ValueRef::String(s))),
            Self::ReceivedAt => predicate(&ValueRef::Number(facts.received_at)),
            Self::Size => predicate(&ValueRef::Unsigned(facts.size)),
            Self::AttachmentCount => predicate(&ValueRef::Unsigned(facts.attachment_count)),
            Self::HasAttachments => predicate(&ValueRef::Bool(facts.attachment_count > 0)),
            Self::Header(name) => facts
                .headers
                .get(name)
                .is_some_and(|vs| vs.iter().any(|s| predicate(&ValueRef::String(s)))),
        }
    }
}

enum ValueRef<'a> {
    String(&'a str),
    Number(i64),
    Unsigned(u64),
    Bool(bool),
}

fn compare_ref(actual: &ValueRef<'_>, expected: &Value, op: Operator, field: &Field) -> bool {
    let ordering = match (actual, expected) {
        (ValueRef::String(a), Value::String(b))
            if is_mail_field(field) && matches!(op, Operator::Eq | Operator::Ne) =>
        {
            Some(if a.eq_ignore_ascii_case(b) {
                Ordering::Equal
            } else {
                Ordering::Greater
            })
        }
        (ValueRef::String(a), Value::String(b)) => Some(a.cmp(&b.as_str())),
        (ValueRef::Number(a), Value::Number(b)) => Some(a.cmp(b)),
        (ValueRef::Unsigned(a), Value::Number(b)) => Some(if *b < 0 {
            Ordering::Greater
        } else {
            a.cmp(&(*b as u64))
        }),
        (ValueRef::Bool(a), Value::Bool(b)) => Some(a.cmp(b)),
        _ => None,
    };
    match op {
        Operator::Eq => ordering == Some(Ordering::Equal),
        Operator::Ne => ordering.is_some_and(|o| o != Ordering::Equal),
        Operator::Gt => ordering == Some(Ordering::Greater),
        Operator::Ge => ordering.is_some_and(|o| o != Ordering::Less),
        Operator::Lt => ordering == Some(Ordering::Less),
        Operator::Le => ordering.is_some_and(|o| o != Ordering::Greater),
        Operator::Contains => {
            matches!((actual, expected), (ValueRef::String(a), Value::String(b)) if if is_mail_field(field) { contains_ascii_case_insensitive(a, b) } else { a.contains(b.as_str()) })
        }
        Operator::StartsWith => {
            matches!((actual, expected), (ValueRef::String(a), Value::String(b)) if if is_mail_field(field) { starts_ascii_case_insensitive(a, b) } else { a.starts_with(b.as_str()) })
        }
        Operator::EndsWith => {
            matches!((actual, expected), (ValueRef::String(a), Value::String(b)) if if is_mail_field(field) { ends_ascii_case_insensitive(a, b) } else { a.ends_with(b.as_str()) })
        }
        Operator::Matches => {
            matches!((actual, expected), (ValueRef::String(a), Value::String(b)) if glob_matches(a, b, is_mail_field(field)))
        }
    }
}

fn is_mail_field(field: &Field) -> bool {
    matches!(
        field,
        Field::EnvelopeFromAddress
            | Field::EnvelopeFromDomain
            | Field::EnvelopeToAddress
            | Field::EnvelopeToDomain
            | Field::FromAddress
            | Field::FromDomain
            | Field::ToAddress
            | Field::ToDomain
            | Field::CcAddress
            | Field::CcDomain
    )
}

fn contains_ascii_case_insensitive(text: &str, needle: &str) -> bool {
    needle.is_empty()
        || text
            .as_bytes()
            .windows(needle.len())
            .any(|part| part.eq_ignore_ascii_case(needle.as_bytes()))
}

fn starts_ascii_case_insensitive(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|s| s.eq_ignore_ascii_case(prefix))
}

fn ends_ascii_case_insensitive(text: &str, suffix: &str) -> bool {
    text.len()
        .checked_sub(suffix.len())
        .and_then(|i| text.get(i..))
        .is_some_and(|s| s.eq_ignore_ascii_case(suffix))
}

// Greedy matching is total; worst case is O(text length * pattern length), with pattern length bounded by MAX_SOURCE.
fn glob_matches(text: &str, pattern: &str, ascii_case_insensitive: bool) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    if p.iter().all(|c| *c == '*') {
        return true;
    }
    let (mut i, mut j, mut star, mut retry) = (0, 0, None, 0usize);
    while i < text.len() {
        let ch = text[i..].chars().next().unwrap();
        if j < p.len() && p[j] == '*' {
            star = Some(j);
            j += 1;
            retry = i;
        } else if j < p.len()
            && (p[j] == '?'
                || if ascii_case_insensitive {
                    p[j].eq_ignore_ascii_case(&ch)
                } else {
                    p[j] == ch
                })
        {
            i += ch.len_utf8();
            j += 1;
        } else if let Some(k) = star {
            if retry >= text.len() {
                return false;
            }
            retry += text[retry..].chars().next().unwrap().len_utf8();
            i = retry;
            j = k + 1;
        } else {
            return false;
        }
    }
    p[j..].iter().all(|c| *c == '*')
}

#[derive(Debug, Clone)]
struct Token {
    kind: Kind,
    start: usize,
}
#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Word(String),
    String(String),
    Number(i64),
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dot,
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    End,
}

fn lex(source: &str) -> Result<Vec<Token>, QueryError> {
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        let simple = match bytes[i] {
            b'(' => Some(Kind::LParen),
            b')' => Some(Kind::RParen),
            b'[' => Some(Kind::LBracket),
            b']' => Some(Kind::RBracket),
            b'.' => Some(Kind::Dot),
            b'=' if bytes.get(i + 1) == Some(&b'=') => {
                i += 1;
                Some(Kind::Eq)
            }
            b'!' if bytes.get(i + 1) == Some(&b'=') => {
                i += 1;
                Some(Kind::Ne)
            }
            b'>' if bytes.get(i + 1) == Some(&b'=') => {
                i += 1;
                Some(Kind::Ge)
            }
            b'>' => Some(Kind::Gt),
            b'<' if bytes.get(i + 1) == Some(&b'=') => {
                i += 1;
                Some(Kind::Le)
            }
            b'<' => Some(Kind::Lt),
            _ => None,
        };
        let kind = if let Some(k) = simple {
            i += 1;
            k
        } else if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            let mut value = String::new();
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' {
                    i += 1;
                    if i >= bytes.len() {
                        return Err(QueryError::at(start, "unterminated escape"));
                    }
                    let escaped = source[i..].chars().next().unwrap();
                    value.push(match escaped {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        other => other,
                    });
                    i += escaped.len_utf8();
                } else {
                    let c = source[i..].chars().next().unwrap();
                    value.push(c);
                    i += c.len_utf8();
                }
            }
            if i == bytes.len() {
                return Err(QueryError::at(start, "unterminated string"));
            }
            i += 1;
            Kind::String(value)
        } else if bytes[i].is_ascii_digit()
            || (bytes[i] == b'-' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let n = source[start..i]
                .parse()
                .map_err(|_| QueryError::at(start, "number is outside the supported range"))?;
            Kind::Number(n)
        } else if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'-'))
            {
                i += 1;
            }
            Kind::Word(source[start..i].to_ascii_lowercase())
        } else {
            return Err(QueryError::at(i, "unexpected character"));
        };
        out.push(Token { kind, start });
        if out.len() > MAX_TOKENS {
            return Err(QueryError::at(start, "query exceeds maximum token count"));
        }
    }
    out.push(Token {
        kind: Kind::End,
        start: source.len(),
    });
    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
    depth: usize,
}
impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.cursor).filter(|t| t.kind != Kind::End)
    }
    fn take(&mut self) -> Token {
        let t = self.tokens[self.cursor].clone();
        if t.kind != Kind::End {
            self.cursor += 1;
        }
        t
    }
    fn parse_or(&mut self) -> Result<Expr, QueryError> {
        let mut xs = vec![self.parse_and()?];
        while self.word("or") {
            self.take();
            xs.push(self.parse_and()?);
        }
        Ok(if xs.len() == 1 {
            xs.pop().unwrap()
        } else {
            Expr::Or(xs)
        })
    }
    fn parse_and(&mut self) -> Result<Expr, QueryError> {
        let mut xs = vec![self.parse_not()?];
        while self.word("and") {
            self.take();
            xs.push(self.parse_not()?);
        }
        Ok(if xs.len() == 1 {
            xs.pop().unwrap()
        } else {
            Expr::And(xs)
        })
    }
    fn parse_not(&mut self) -> Result<Expr, QueryError> {
        if self.word("not") {
            let pos = self.take().start;
            self.depth += 1;
            if self.depth > MAX_DEPTH {
                return Err(QueryError::at(pos, "maximum expression nesting exceeded"));
            }
            let nested = self.parse_not()?;
            self.depth -= 1;
            return Ok(Expr::Not(Box::new(nested)));
        }
        if self.word("true") {
            self.take();
            return Ok(Expr::True);
        }
        if self.word("false") {
            self.take();
            return Ok(Expr::False);
        }
        if self.peek().is_some_and(|t| t.kind == Kind::LParen) {
            let pos = self.take().start;
            self.depth += 1;
            if self.depth > MAX_DEPTH {
                return Err(QueryError::at(pos, "maximum parenthesis nesting exceeded"));
            }
            let e = self.parse_or()?;
            if self.peek().is_none_or(|t| t.kind != Kind::RParen) {
                return Err(QueryError::at(
                    self.tokens[self.cursor].start,
                    "expected ')'",
                ));
            }
            self.take();
            self.depth -= 1;
            return Ok(e);
        }
        self.parse_predicate()
    }
    fn parse_predicate(&mut self) -> Result<Expr, QueryError> {
        let start = self.tokens[self.cursor].start;
        let field = self.parse_field()?;
        let t = self.take();
        let op = match t.kind {
            Kind::Eq => Operator::Eq,
            Kind::Ne => Operator::Ne,
            Kind::Gt => Operator::Gt,
            Kind::Ge => Operator::Ge,
            Kind::Lt => Operator::Lt,
            Kind::Le => Operator::Le,
            Kind::Word(w) => match w.as_str() {
                "contains" => Operator::Contains,
                "starts_with" => Operator::StartsWith,
                "ends_with" => Operator::EndsWith,
                "matches" => Operator::Matches,
                _ => return Err(QueryError::at(t.start, "expected comparison operator")),
            },
            _ => return Err(QueryError::at(t.start, "expected comparison operator")),
        };
        let value_token = self.take();
        let mut value = match value_token.kind {
            Kind::String(s) => Value::String(s),
            Kind::Number(n) => Value::Number(n),
            Kind::Word(ref w) if w == "true" => Value::Bool(true),
            Kind::Word(ref w) if w == "false" => Value::Bool(false),
            _ => {
                return Err(QueryError::at(
                    value_token.start,
                    "expected string, number, or boolean literal",
                ));
            }
        };
        let valid = match (&field, &value, op) {
            (
                Field::ReceivedAt | Field::Size | Field::AttachmentCount,
                Value::Number(_),
                Operator::Eq
                | Operator::Ne
                | Operator::Gt
                | Operator::Ge
                | Operator::Lt
                | Operator::Le,
            ) => true,
            (Field::HasAttachments, Value::Bool(_), Operator::Eq | Operator::Ne) => true,
            (
                _,
                Value::String(_),
                Operator::Eq
                | Operator::Ne
                | Operator::Contains
                | Operator::StartsWith
                | Operator::EndsWith
                | Operator::Matches,
            ) => !matches!(
                field,
                Field::ReceivedAt | Field::Size | Field::AttachmentCount | Field::HasAttachments
            ),
            (_, Value::String(_), Operator::Gt | Operator::Ge | Operator::Lt | Operator::Le) => {
                matches!(
                    field,
                    Field::Subject
                        | Field::Text
                        | Field::Html
                        | Field::MessageId
                        | Field::Header(_)
                )
            }
            _ => false,
        };
        if !valid {
            return Err(QueryError::at(
                value_token.start.max(start),
                "operator and literal type do not match this field",
            ));
        }
        if is_mail_field(&field)
            && let Value::String(s) = &mut value
        {
            *s = s.to_ascii_lowercase();
        }
        Ok(Expr::Predicate(Predicate { field, op, value }))
    }
    fn parse_field(&mut self) -> Result<Field, QueryError> {
        let first = self.take();
        let name = match first.kind {
            Kind::Word(w) => w,
            _ => return Err(QueryError::at(first.start, "expected field name")),
        };
        if name == "header" {
            if self.peek().is_none_or(|t| t.kind != Kind::LBracket) {
                return Err(QueryError::at(first.start, "expected header[\"name\"]"));
            }
            self.take();
            let key = self.take();
            let mut header = match key.kind {
                Kind::String(s) => s.to_ascii_lowercase(),
                _ => return Err(QueryError::at(key.start, "header name must be a string")),
            };
            if self.peek().is_none_or(|t| t.kind != Kind::RBracket) {
                return Err(QueryError::at(
                    self.tokens[self.cursor].start,
                    "expected ']'",
                ));
            }
            self.take();
            header = header.trim().to_owned();
            return Ok(Field::Header(header));
        }
        let mut full = name;
        while self.peek().is_some_and(|t| t.kind == Kind::Dot) {
            self.take();
            let part = self.take();
            match part.kind {
                Kind::Word(w) => {
                    full.push('.');
                    full.push_str(&w);
                }
                _ => return Err(QueryError::at(part.start, "expected field component")),
            }
        }
        match full.as_str() {
            "envelope.from.address" | "envelope.from" => Ok(Field::EnvelopeFromAddress),
            "envelope.from.domain" => Ok(Field::EnvelopeFromDomain),
            "envelope.to.address" | "envelope.to" => Ok(Field::EnvelopeToAddress),
            "envelope.to.domain" => Ok(Field::EnvelopeToDomain),
            "from.address" | "from" => Ok(Field::FromAddress),
            "from.domain" => Ok(Field::FromDomain),
            "to.address" | "to" => Ok(Field::ToAddress),
            "to.domain" => Ok(Field::ToDomain),
            "cc.address" | "cc" => Ok(Field::CcAddress),
            "cc.domain" => Ok(Field::CcDomain),
            "subject" => Ok(Field::Subject),
            "text" => Ok(Field::Text),
            "html" => Ok(Field::Html),
            "message_id" => Ok(Field::MessageId),
            "received_at" => Ok(Field::ReceivedAt),
            "size" => Ok(Field::Size),
            "attachment_count" => Ok(Field::AttachmentCount),
            "has_attachments" => Ok(Field::HasAttachments),
            _ => Err(QueryError::at(first.start, "unknown field")),
        }
    }
    fn word(&self, expected: &str) -> bool {
        self.peek()
            .is_some_and(|t| matches!(&t.kind, Kind::Word(w) if w == expected))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandpost_core::Mailbox;

    fn facts() -> MessageFacts {
        MessageFacts {
            from: vec![Mailbox {
                address: "a@example.com".into(),
                domain: "example.com".into(),
            }],
            to: vec![
                Mailbox {
                    address: "b@test.org".into(),
                    domain: "test.org".into(),
                },
                Mailbox {
                    address: "c@other.org".into(),
                    domain: "other.org".into(),
                },
            ],
            cc: vec![],
            subject: "Hello, world".into(),
            text: "A quoted \"word\"".into(),
            received_at: 123,
            size: 42,
            attachment_count: 1,
            headers: [("x-tag".into(), vec!["blue".into(), "green".into()])].into(),
            ..Default::default()
        }
    }

    #[test]
    fn precedence_literals_and_escapes() {
        let q = compile("not subject == 'no' and (from.domain == \"example.COM\" or size >= 50) and text contains \"quoted \\\"word\\\"\"").unwrap();
        assert!(q.evaluate(&facts()));
    }

    #[test]
    fn recipient_any_not_equal_and_headers() {
        assert!(
            compile("to.address != \"absent@test.org\"")
                .unwrap()
                .evaluate(&facts())
        );
        assert!(
            compile("to.address != \"b@test.org\"")
                .unwrap()
                .evaluate(&facts())
        );
        assert!(
            compile("to.address == \"B@TEST.ORG\"")
                .unwrap()
                .evaluate(&facts())
        );
        assert!(
            compile("header[\"X-TAG\"] == \"green\"")
                .unwrap()
                .evaluate(&facts())
        );
    }

    #[test]
    fn glob_normalization_and_fingerprint() {
        assert!(
            compile("subject matches \"H*?o*\"")
                .unwrap()
                .evaluate(&facts())
        );
        assert_eq!(
            compile("size > 1 and subject == \"Hello, world\"")
                .unwrap()
                .fingerprint(),
            compile("subject == \"Hello, world\" AND size > 1")
                .unwrap()
                .fingerprint()
        );
        assert_eq!(
            compile("size > 1 and (subject == \"Hello, world\" and text contains \"quoted\")")
                .unwrap()
                .fingerprint(),
            compile("text contains \"quoted\" and size > 1 and subject == \"Hello, world\"")
                .unwrap()
                .fingerprint()
        );
        assert_eq!(
            compile("not not subject == \"Hello, world\"")
                .unwrap()
                .fingerprint(),
            compile("subject == \"Hello, world\"")
                .unwrap()
                .fingerprint()
        );
        assert!(matches!(
            compile("subject == \"Hello, world\" and not subject == \"Hello, world\"")
                .unwrap()
                .expression,
            Expr::False
        ));
        assert!(!compile("not true and false").unwrap().evaluate(&facts()));
        assert!(
            !compile("false or true and false")
                .unwrap()
                .evaluate(&facts())
        );
        assert!(
            compile("true or false and false")
                .unwrap()
                .evaluate(&facts())
        );
        let Expr::Predicate(p) = &compile("from.address == \"A@EXAMPLE.COM\"")
            .unwrap()
            .expression
        else {
            panic!()
        };
        assert_eq!(p.value, Value::String("a@example.com".into()));
        assert_eq!(
            Field::FromAddress.values(&facts()),
            vec![Value::String("a@example.com".into())]
        );
        let mut with_star = facts();
        with_star.text = "*xa".into();
        assert!(compile("text matches \"*a\"").unwrap().evaluate(&with_star));
        assert_eq!(compile("a").unwrap_err().position, 0);
    }

    #[test]
    fn empty_constants_and_safety_limits() {
        assert!(compile(" ").unwrap().evaluate(&facts()));
        assert!(compile("true").unwrap().evaluate(&facts()));
        assert!(!compile("false").unwrap().evaluate(&facts()));
        assert!(compile("true == true").is_err());
        let nested = "(".repeat(MAX_DEPTH + 1) + "true == true" + &")".repeat(MAX_DEPTH + 1);
        assert!(compile(&nested).is_err());
        assert!(compile("subject == \"x\" and").is_err());
    }

    #[test]
    fn type_errors_and_boolean_simplification() {
        for source in [
            "size contains 'x'",
            "has_attachments > false",
            "from.domain == 12",
            "unknown_field == 'x'",
            "subject == true",
        ] {
            assert!(compile(source).is_err(), "accepted {source}");
        }

        let a = "subject == 'Hello, world'";
        let not_a = format!("not ({a})");
        for equivalent in [format!("{a} and true"), format!("not not ({a})")] {
            assert_eq!(
                compile(a).unwrap().expression,
                compile(&equivalent).unwrap().expression
            );
        }
        assert_eq!(
            compile(&format!("{a} and {not_a}")).unwrap().expression,
            Expr::False
        );
        assert_eq!(
            compile(&format!("{a} or {not_a}")).unwrap().expression,
            Expr::True
        );

        let mixed = facts();
        assert!(
            compile("to.address == 'b@test.org'")
                .unwrap()
                .evaluate(&mixed)
        );
        assert!(
            compile("to.address != 'b@test.org'")
                .unwrap()
                .evaluate(&mixed)
        );
    }

    #[test]
    fn malformed_and_truncated_sources_never_panic() {
        let samples = [
            "subject ==",
            "header[",
            "header[\"x\"",
            "header[true]",
            "header[\"x\"]",
            "(",
            "subject",
            ")",
            "and",
            "subject == \"x\" or",
            "from.",
            "envelope.to.domain >",
            "subject === 'x'",
            "subject == 'unterminated",
            "header[\"x\"] ==",
            "((subject == \"x\")",
            "not not",
            "size >= -",
        ];
        for sample in samples {
            for end in 0..=sample.len() {
                let prefix = &sample[..end];
                assert!(
                    std::panic::catch_unwind(|| compile(prefix)).is_ok(),
                    "panicked on {prefix:?}"
                );
            }
        }
    }

    #[test]
    fn wildcard_handles_large_text_without_semantic_cutoff() {
        let mut large = facts();
        large.text = "x".repeat(1_000_123);
        assert!(compile("text matches \"*\"").unwrap().evaluate(&large));
    }
}
