//! The typed syntax tree of a parsed script and the limits applied while parsing it.
use crate::diagnostics::Span;
use sandpost_query::CompiledQuery;
use std::fmt;
use std::time::Duration;

/// Bounds that keep scripts small, shallow, and quick to finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptLimits {
    /// Largest accepted source, in UTF-8 bytes.
    pub maximum_source_bytes: usize,
    /// Most statements in the whole script, counting nested statements and `if` statements.
    pub maximum_statements: usize,
    /// Deepest nesting of blocks; a top-level `if` block is depth one.
    pub maximum_nesting_depth: usize,
    /// Longest single `wait`.
    pub maximum_wait: Duration,
    /// Longest sum of waits along any path through the script.
    pub maximum_total_wait: Duration,
}

impl Default for ScriptLimits {
    /**
     * Allow 16 KiB, 256 statements, 16 nested blocks, ten-minute waits and thirty minutes of
     * waiting along any path.
     */
    fn default() -> Self {
        Self {
            maximum_source_bytes: 16 * 1024,
            maximum_statements: 256,
            maximum_nesting_depth: 16,
            maximum_wait: Duration::from_secs(10 * 60),
            maximum_total_wait: Duration::from_secs(30 * 60),
        }
    }
}

/// A parsed script: its top-level statements in source order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    pub statements: Vec<Statement>,
}

/// One statement and the source it was parsed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub kind: StatementKind,
    pub span: Span,
}

/// What a statement does.
///
/// New statement kinds, such as bounded loops, extend this enum and the lowering in
/// [`crate::compile`]; existing kinds and their instructions stay unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatementKind {
    /// `open;` — open the message as a mail client would, loading its remote images.
    Open,
    /// `wait 5s;` — pause before the next statement.
    Wait(Duration),
    /// `click link(...);` — follow one link of the message's HTML.
    Click(LinkSelector),
    /// `if (condition) { ... } else ...` — run one branch depending on a query condition.
    If(Conditional),
}

/// An `if` statement with its optional `else` or `else if` continuation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conditional {
    pub condition: Condition,
    pub consequence: Vec<Statement>,
    pub alternative: Option<Alternative>,
}

/// What follows `else`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Alternative {
    /// `else if (...) { ... }`: the nested statement is always an `if`.
    ElseIf(Box<Statement>),
    /// `else { ... }`.
    Else(Vec<Statement>),
}

/// A condition compiled by `sandpost-query`, with the source text it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    /// The condition as written between the parentheses, trimmed.
    pub source: String,
    /// The canonical query evaluated against the message's facts.
    pub query: CompiledQuery,
    pub span: Span,
}

impl fmt::Display for Condition {
    /**
     * Write the condition as written in the script.
     */
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.source)
    }
}

/// How `click` chooses a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkSelector {
    /// `link("a.verify")`: the first link matched by a CSS selector.
    Css(CssSelector),
    /// `link(text: "Confirm*")`: the first link whose visible text matches a glob, ignoring
    /// ASCII case.
    Text(String),
    /// `link(href: "*unsubscribe*")`: the first link whose resolved URL matches a glob.
    Href(String),
}

/// A validated CSS selector and its source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssSelector {
    source: String,
    selector: scraper::Selector,
}

impl CssSelector {
    /**
     * Parse a CSS selector, returning the parser's explanation when it is invalid.
     */
    pub fn parse(source: &str) -> Result<Self, String> {
        scraper::Selector::parse(source)
            .map(|selector| Self {
                source: source.to_owned(),
                selector,
            })
            .map_err(|error| error.to_string())
    }

    /**
     * Return the selector as written in the script.
     */
    pub fn source(&self) -> &str {
        &self.source
    }

    /**
     * Return the compiled selector used to match elements.
     */
    pub fn selector(&self) -> &scraper::Selector {
        &self.selector
    }
}

impl fmt::Display for LinkSelector {
    /**
     * Write the selector in PersonaScript syntax, such as `link(text: "Confirm")`.
     */
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (prefix, value) = match self {
            Self::Css(selector) => ("", selector.source()),
            Self::Text(text) => ("text: ", text.as_str()),
            Self::Href(pattern) => ("href: ", pattern.as_str()),
        };
        write!(formatter, "link({prefix}\"")?;
        for character in value.chars() {
            match character {
                '"' => formatter.write_str("\\\"")?,
                '\\' => formatter.write_str("\\\\")?,
                '\n' => formatter.write_str("\\n")?,
                '\r' => formatter.write_str("\\r")?,
                '\t' => formatter.write_str("\\t")?,
                other => write!(formatter, "{other}")?,
            }
        }
        formatter.write_str("\")")
    }
}
