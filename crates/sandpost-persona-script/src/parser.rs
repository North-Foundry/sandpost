//! Recursive-descent parsing of PersonaScript into the typed syntax tree.
//!
//! Grammar (`CONDITION` is the Sandpost query language, compiled by `sandpost-query`):
//!
//! ```text
//! script        = { statement } END
//! statement     = "open" ";"
//!               | "wait" DURATION ";"
//!               | "click" link_selector ";"
//!               | if_statement
//! if_statement  = "if" "(" CONDITION ")" block [ "else" ( if_statement | block ) ]
//! block         = "{" { statement } "}"
//! link_selector = "link" "(" ( STRING | ( "text" | "href" ) ":" STRING ) ")"
//! DURATION      = DIGITS ( "ms" | "s" | "m" )
//! ```
use crate::diagnostics::{Diagnostic, ScriptError, SourceMap};
use crate::lexer::{Lexer, Token, TokenKind};
use crate::syntax::{
    Alternative, Condition, Conditional, CssSelector, LinkSelector, Script, ScriptLimits,
    Statement, StatementKind,
};
use std::time::Duration;

/// Longest CSS selector, link text, or URL pattern accepted in `link(...)`, in bytes.
const MAXIMUM_ARGUMENT_BYTES: usize = 1024;

/**
 * Parse and validate a script, returning the first problem with its line and column.
 *
 * Conditions are compiled by `sandpost-query`, CSS selectors are parsed, and every limit in
 * `limits` is enforced, so a successful parse can always be lowered to a [`crate::Program`].
 */
pub fn parse(source: &str, limits: &ScriptLimits) -> Result<Script, ScriptError> {
    let map = SourceMap::new(source);
    if source.len() > limits.maximum_source_bytes {
        return Err(map.error(Diagnostic::at(
            limits.maximum_source_bytes,
            format!(
                "script exceeds the maximum size of {} bytes",
                limits.maximum_source_bytes
            ),
        )));
    }
    let mut parser = Parser {
        source,
        map: &map,
        lexer: Lexer::new(source),
        peeked: None,
        limits,
        statement_count: 0,
    };
    parser
        .parse_script()
        .map_err(|diagnostic| map.error(diagnostic))
}

/// Parsing state for one source.
struct Parser<'source, 'map> {
    source: &'source str,
    map: &'map SourceMap<'source>,
    lexer: Lexer<'source>,
    peeked: Option<Token>,
    limits: &'map ScriptLimits,
    statement_count: usize,
}

impl Parser<'_, '_> {
    /**
     * Parse statements until the end of the source.
     */
    fn parse_script(&mut self) -> Result<Script, Diagnostic> {
        let mut statements = Vec::new();
        loop {
            let token = self.peek()?;
            match token.kind {
                TokenKind::End => return Ok(Script { statements }),
                TokenKind::RightBrace => {
                    return Err(Diagnostic::at(
                        token.start,
                        "unexpected `}` without a matching `{`",
                    ));
                }
                _ => statements.push(self.parse_statement(0)?),
            }
        }
    }

    /**
     * Parse one statement inside blocks nested `depth` levels deep.
     */
    fn parse_statement(&mut self, depth: usize) -> Result<Statement, Diagnostic> {
        let token = self.next()?;
        self.statement_count += 1;
        if self.statement_count > self.limits.maximum_statements {
            return Err(Diagnostic::at(
                token.start,
                format!(
                    "script exceeds the maximum of {} statements",
                    self.limits.maximum_statements
                ),
            ));
        }
        let word = match &token.kind {
            TokenKind::Identifier(word) => word.clone(),
            TokenKind::Semicolon => {
                return Err(Diagnostic::at(
                    token.start,
                    "unexpected `;`; empty statements are not allowed",
                ));
            }
            TokenKind::LeftBrace => {
                return Err(Diagnostic::at(
                    token.start,
                    "unexpected `{`; a block can only follow `if (...)` or `else`",
                ));
            }
            _ => {
                return Err(Diagnostic::at(
                    token.start,
                    "expected an instruction: `open`, `wait`, `click` or `if`",
                ));
            }
        };
        let kind = match word.as_str() {
            "open" => StatementKind::Open,
            "wait" => StatementKind::Wait(self.parse_wait_duration()?),
            "click" => StatementKind::Click(self.parse_link_selector()?),
            "if" => return self.parse_if(token, depth),
            "else" => {
                return Err(Diagnostic::at(
                    token.start,
                    "`else` must directly follow the `}` that closes an `if` block",
                ));
            }
            other => {
                return Err(Diagnostic::at(
                    token.start,
                    format!(
                        "unknown instruction `{other}`; expected `open`, `wait`, `click` or `if`"
                    ),
                ));
            }
        };
        let end = self.expect_semicolon(&word)?;
        Ok(Statement {
            kind,
            span: self.map.span(token.start, end),
        })
    }

    /**
     * Parse the duration after `wait` and check it against the single-wait limit.
     */
    fn parse_wait_duration(&mut self) -> Result<Duration, Diagnostic> {
        let token = self.next()?;
        let TokenKind::Duration(duration) = token.kind else {
            return Err(Diagnostic::at(
                token.start,
                "expected a duration after `wait`, such as `500ms`, `5s` or `2m`",
            ));
        };
        if duration > self.limits.maximum_wait {
            return Err(Diagnostic::at(
                token.start,
                format!(
                    "wait of {} exceeds the maximum of {}",
                    describe_duration(duration),
                    describe_duration(self.limits.maximum_wait)
                ),
            ));
        }
        Ok(duration)
    }

    /**
     * Parse `link(...)` after `click`.
     */
    fn parse_link_selector(&mut self) -> Result<LinkSelector, Diagnostic> {
        let token = self.next()?;
        if token.kind != TokenKind::Identifier("link".into()) {
            return Err(Diagnostic::at(
                token.start,
                "expected `link(...)` after `click`, as in `click link(\"a.verify\")`",
            ));
        }
        self.expect(TokenKind::LeftParenthesis, "expected `(` after `link`")?;
        let argument = self.next()?;
        let selector = match argument.kind {
            TokenKind::String(source) => {
                validate_argument(&source, argument.start, "CSS selector")?;
                LinkSelector::Css(CssSelector::parse(&source).map_err(|error| {
                    Diagnostic::at(argument.start, format!("invalid CSS selector: {error}"))
                })?)
            }
            TokenKind::Identifier(name) if name == "text" || name == "href" => {
                self.expect(TokenKind::Colon, &format!("expected `:` after `{name}`"))?;
                let value_token = self.next()?;
                let TokenKind::String(value) = value_token.kind else {
                    return Err(Diagnostic::at(
                        value_token.start,
                        format!("expected a quoted string after `{name}:`"),
                    ));
                };
                if name == "text" {
                    validate_argument(&value, value_token.start, "link text")?;
                    LinkSelector::Text(value)
                } else {
                    validate_argument(&value, value_token.start, "URL pattern")?;
                    LinkSelector::Href(value)
                }
            }
            TokenKind::Identifier(name) => {
                return Err(Diagnostic::at(
                    argument.start,
                    format!(
                        "unknown link argument `{name}`; use a CSS selector string, `text:` or `href:`"
                    ),
                ));
            }
            TokenKind::RightParenthesis => {
                return Err(Diagnostic::at(
                    argument.start,
                    "`link()` needs a CSS selector string, `text:` or `href:`",
                ));
            }
            _ => {
                return Err(Diagnostic::at(
                    argument.start,
                    "expected a CSS selector string, `text:` or `href:`",
                ));
            }
        };
        let closing = self.next()?;
        match closing.kind {
            TokenKind::RightParenthesis => Ok(selector),
            TokenKind::Comma => Err(Diagnostic::at(
                closing.start,
                "`link(...)` accepts exactly one argument",
            )),
            _ => Err(Diagnostic::at(
                closing.start,
                "expected `)` to close `link(`",
            )),
        }
    }

    /**
     * Parse an `if` statement whose keyword has been consumed, including `else` branches.
     */
    fn parse_if(&mut self, keyword: Token, depth: usize) -> Result<Statement, Diagnostic> {
        let opening = self.expect(TokenKind::LeftParenthesis, "expected `(` after `if`")?;
        let embedded =
            sandpost_query::compile_embedded(self.source, opening.end).map_err(|error| {
                Diagnostic::at(
                    error.position,
                    format!("invalid condition: {}", error.message),
                )
            })?;
        let condition_source = self.source[opening.end..embedded.end].trim().to_owned();
        let condition = Condition {
            source: condition_source,
            query: embedded.query,
            span: self.map.span(opening.end, embedded.end),
        };
        self.peeked = None;
        self.lexer.seek(embedded.end + 1);
        let (consequence, mut end) = self.parse_block(depth + 1, "if")?;
        let alternative = if matches!(&self.peek()?.kind, TokenKind::Identifier(word) if word == "else")
        {
            self.next()?;
            let next = self.peek()?;
            if next.kind == TokenKind::Identifier("if".into()) {
                let nested_keyword = self.next()?;
                self.statement_count += 1;
                if self.statement_count > self.limits.maximum_statements {
                    return Err(Diagnostic::at(
                        nested_keyword.start,
                        format!(
                            "script exceeds the maximum of {} statements",
                            self.limits.maximum_statements
                        ),
                    ));
                }
                let nested = self.parse_if(nested_keyword, depth)?;
                end = nested.span.end;
                Some(Alternative::ElseIf(Box::new(nested)))
            } else {
                let (statements, block_end) = self.parse_block(depth + 1, "else")?;
                end = block_end;
                Some(Alternative::Else(statements))
            }
        } else {
            None
        };
        Ok(Statement {
            kind: StatementKind::If(Conditional {
                condition,
                consequence,
                alternative,
            }),
            span: self.map.span(keyword.start, end),
        })
    }

    /**
     * Parse a braced block at nesting `depth`, returning its statements and end offset.
     */
    fn parse_block(
        &mut self,
        depth: usize,
        owner: &str,
    ) -> Result<(Vec<Statement>, usize), Diagnostic> {
        let opening = self.next()?;
        if opening.kind != TokenKind::LeftBrace {
            return Err(Diagnostic::at(
                opening.start,
                format!("expected `{{` to start the `{owner}` block"),
            ));
        }
        if depth > self.limits.maximum_nesting_depth {
            return Err(Diagnostic::at(
                opening.start,
                format!(
                    "blocks are nested deeper than the maximum of {} levels",
                    self.limits.maximum_nesting_depth
                ),
            ));
        }
        let mut statements = Vec::new();
        loop {
            let token = self.peek()?;
            match token.kind {
                TokenKind::RightBrace => {
                    let closing = self.next()?;
                    return Ok((statements, closing.end));
                }
                TokenKind::End => {
                    let (line, column) = self.map.location(opening.start);
                    return Err(Diagnostic::at(
                        token.start,
                        format!(
                            "expected `}}` to close the block opened at line {line}, column {column}"
                        ),
                    ));
                }
                _ => statements.push(self.parse_statement(depth)?),
            }
        }
    }

    /**
     * Require the `;` that ends a simple statement and return its end offset.
     */
    fn expect_semicolon(&mut self, instruction: &str) -> Result<usize, Diagnostic> {
        let token = self.peek()?;
        if token.kind == TokenKind::Semicolon {
            self.next()?;
            Ok(token.end)
        } else {
            Err(Diagnostic::at(
                token.start,
                format!("expected `;` after `{instruction}`"),
            ))
        }
    }

    /**
     * Consume a token of the expected kind or fail with `message` at the unexpected token.
     */
    fn expect(&mut self, kind: TokenKind, message: &str) -> Result<Token, Diagnostic> {
        let token = self.next()?;
        if token.kind == kind {
            Ok(token)
        } else {
            Err(Diagnostic::at(token.start, message))
        }
    }

    /**
     * Return the next token without consuming it.
     */
    fn peek(&mut self) -> Result<Token, Diagnostic> {
        if self.peeked.is_none() {
            self.peeked = Some(self.lexer.next_token()?);
        }
        Ok(self.peeked.clone().expect("peeked token"))
    }

    /**
     * Consume and return the next token.
     */
    fn next(&mut self) -> Result<Token, Diagnostic> {
        match self.peeked.take() {
            Some(token) => Ok(token),
            None => self.lexer.next_token(),
        }
    }
}

/**
 * Reject empty or oversized `link(...)` arguments.
 */
fn validate_argument(value: &str, position: usize, description: &str) -> Result<(), Diagnostic> {
    if value.trim().is_empty() {
        return Err(Diagnostic::at(
            position,
            format!("the {description} must not be empty"),
        ));
    }
    if value.len() > MAXIMUM_ARGUMENT_BYTES {
        return Err(Diagnostic::at(
            position,
            format!("the {description} exceeds the maximum of {MAXIMUM_ARGUMENT_BYTES} bytes"),
        ));
    }
    Ok(())
}

/**
 * Describe a duration in the largest PersonaScript unit that represents it exactly.
 */
pub(crate) fn describe_duration(duration: Duration) -> String {
    let milliseconds = duration.as_millis();
    if milliseconds.is_multiple_of(60_000) && milliseconds > 0 {
        format!("{}m", milliseconds / 60_000)
    } else if milliseconds.is_multiple_of(1000) {
        format!("{}s", milliseconds / 1000)
    } else {
        format!("{milliseconds}ms")
    }
}
