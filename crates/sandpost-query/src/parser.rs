//! Grammar, precedence, and predicate type validation.
use crate::{
    compiler::QueryError,
    expression::{Expression, Field, Operator, Predicate, Value},
    lexer::{Token, TokenKind},
};

pub(crate) const MAXIMUM_NESTING_DEPTH: usize = 64;

/// Parse tokens into an expression and reject any unconsumed trailing token.
pub(crate) fn parse_tokens(tokens: Vec<Token>) -> Result<Expression, QueryError> {
    let mut parser = Parser {
        tokens,
        token_index: 0,
        nesting_depth: 0,
    };
    let expression = parser.parse_or_expression()?;
    if let Some(token) = parser.peek_token() {
        return Err(QueryError::at_position(
            token.source_position,
            "unexpected token",
        ));
    }
    Ok(expression)
}

struct Parser {
    tokens: Vec<Token>,
    token_index: usize,
    nesting_depth: usize,
}
impl Parser {
    /// Return the next non-sentinel token without advancing the parser.
    fn peek_token(&self) -> Option<&Token> {
        self.tokens
            .get(self.token_index)
            .filter(|token| token.kind != TokenKind::End)
    }
    /// Return the next token and advance unless it is the end sentinel.
    fn consume_token(&mut self) -> Token {
        let token = self.tokens[self.token_index].clone();
        if token.kind != TokenKind::End {
            self.token_index += 1;
        }
        token
    }
    /// Parse one or more AND expressions joined by the lower-precedence OR operator.
    fn parse_or_expression(&mut self) -> Result<Expression, QueryError> {
        let mut expressions = vec![self.parse_and_expression()?];
        while self.next_word_is("or") {
            self.consume_token();
            expressions.push(self.parse_and_expression()?);
        }
        Ok(if expressions.len() == 1 {
            expressions.pop().unwrap()
        } else {
            Expression::Or(expressions)
        })
    }
    /// Parse one or more NOT expressions joined by AND.
    fn parse_and_expression(&mut self) -> Result<Expression, QueryError> {
        let mut expressions = vec![self.parse_not_expression()?];
        while self.next_word_is("and") {
            self.consume_token();
            expressions.push(self.parse_not_expression()?);
        }
        Ok(if expressions.len() == 1 {
            expressions.pop().unwrap()
        } else {
            Expression::And(expressions)
        })
    }
    /// Parse unary negation, boolean constants, parenthesized groups, or predicates.
    fn parse_not_expression(&mut self) -> Result<Expression, QueryError> {
        if self.next_word_is("not") {
            let position = self.consume_token().source_position;
            self.nesting_depth += 1;
            if self.nesting_depth > MAXIMUM_NESTING_DEPTH {
                return Err(QueryError::at_position(
                    position,
                    "maximum expression nesting exceeded",
                ));
            }
            let nested_expression = self.parse_not_expression()?;
            self.nesting_depth -= 1;
            return Ok(Expression::Not(Box::new(nested_expression)));
        }
        if self.next_word_is("true") {
            self.consume_token();
            return Ok(Expression::True);
        }
        if self.next_word_is("false") {
            self.consume_token();
            return Ok(Expression::False);
        }
        if self
            .peek_token()
            .is_some_and(|token| token.kind == TokenKind::LeftParenthesis)
        {
            let position = self.consume_token().source_position;
            self.nesting_depth += 1;
            if self.nesting_depth > MAXIMUM_NESTING_DEPTH {
                return Err(QueryError::at_position(
                    position,
                    "maximum parenthesis nesting exceeded",
                ));
            }
            let expression = self.parse_or_expression()?;
            if self
                .peek_token()
                .is_none_or(|token| token.kind != TokenKind::RightParenthesis)
            {
                return Err(QueryError::at_position(
                    self.tokens[self.token_index].source_position,
                    "expected ')'",
                ));
            }
            self.consume_token();
            self.nesting_depth -= 1;
            return Ok(expression);
        }
        self.parse_predicate()
    }
    /// Parse a field/operator/literal comparison and validate its types.
    fn parse_predicate(&mut self) -> Result<Expression, QueryError> {
        let field = self.parse_field()?;
        let operator = self.parse_operator()?;
        let (mut value, source_position) = self.parse_literal()?;
        if !valid_comparison(&field, operator, &value) {
            return Err(QueryError::at_position(
                source_position,
                "operator and literal type do not match this field",
            ));
        }
        if field.is_ascii_case_insensitive()
            && let Value::String(string) = &mut value
        {
            *string = string.to_ascii_lowercase();
        }
        Ok(Expression::Predicate(Predicate {
            field,
            operator,
            value,
        }))
    }

    /// Parse one supported symbolic or word comparison operator.
    fn parse_operator(&mut self) -> Result<Operator, QueryError> {
        let token = self.consume_token();
        Ok(match token.kind {
            TokenKind::Equal => Operator::Equal,
            TokenKind::NotEqual => Operator::NotEqual,
            TokenKind::GreaterThan => Operator::GreaterThan,
            TokenKind::GreaterThanOrEqual => Operator::GreaterThanOrEqual,
            TokenKind::LessThan => Operator::LessThan,
            TokenKind::LessThanOrEqual => Operator::LessThanOrEqual,
            TokenKind::Word(word) => match word.as_str() {
                "contains" => Operator::Contains,
                "starts_with" => Operator::StartsWith,
                "ends_with" => Operator::EndsWith,
                "matches" => Operator::Matches,
                _ => {
                    return Err(QueryError::at_position(
                        token.source_position,
                        "expected comparison operator",
                    ));
                }
            },
            _ => {
                return Err(QueryError::at_position(
                    token.source_position,
                    "expected comparison operator",
                ));
            }
        })
    }

    /// Parse a string, signed integer, or boolean literal and retain its source position.
    fn parse_literal(&mut self) -> Result<(Value, usize), QueryError> {
        let value_token = self.consume_token();
        let value = match value_token.kind {
            TokenKind::String(string) => Value::String(string),
            TokenKind::Number(number) => Value::Number(number),
            TokenKind::Word(ref word) if word == "true" => Value::Boolean(true),
            TokenKind::Word(ref word) if word == "false" => Value::Boolean(false),
            _ => {
                return Err(QueryError::at_position(
                    value_token.source_position,
                    "expected string, number, or boolean literal",
                ));
            }
        };
        Ok((value, value_token.source_position))
    }

    /// Parse a supported field path or a normalized header lookup.
    fn parse_field(&mut self) -> Result<Field, QueryError> {
        let first_token = self.consume_token();
        let field_name = match first_token.kind {
            TokenKind::Word(word) => word,
            _ => {
                return Err(QueryError::at_position(
                    first_token.source_position,
                    "expected field name",
                ));
            }
        };
        if field_name == "header" {
            if self
                .peek_token()
                .is_none_or(|token| token.kind != TokenKind::LeftBracket)
            {
                return Err(QueryError::at_position(
                    first_token.source_position,
                    "expected header[\"name\"]",
                ));
            }
            self.consume_token();
            let header_token = self.consume_token();
            let mut header_name = match header_token.kind {
                TokenKind::String(string) => string.to_ascii_lowercase(),
                _ => {
                    return Err(QueryError::at_position(
                        header_token.source_position,
                        "header name must be a string",
                    ));
                }
            };
            if self
                .peek_token()
                .is_none_or(|token| token.kind != TokenKind::RightBracket)
            {
                return Err(QueryError::at_position(
                    self.tokens[self.token_index].source_position,
                    "expected ']'",
                ));
            }
            self.consume_token();
            header_name = header_name.trim().to_owned();
            return Ok(Field::Header(header_name));
        }
        let mut full_field_name = field_name;
        while self
            .peek_token()
            .is_some_and(|token| token.kind == TokenKind::Dot)
        {
            self.consume_token();
            let component_token = self.consume_token();
            match component_token.kind {
                TokenKind::Word(component) => {
                    full_field_name.push('.');
                    full_field_name.push_str(&component);
                }
                _ => {
                    return Err(QueryError::at_position(
                        component_token.source_position,
                        "expected field component",
                    ));
                }
            }
        }
        match full_field_name.as_str() {
            "envelope.from.address" | "envelope.from" => Ok(Field::EnvelopeFromAddress),
            "envelope.from.domain" => Ok(Field::EnvelopeFromDomain),
            "envelope.to.address" | "envelope.to" => Ok(Field::EnvelopeToAddress),
            "envelope.to.domain" => Ok(Field::EnvelopeToDomain),
            "from.address" | "from" => Ok(Field::FromAddress),
            "from.domain" => Ok(Field::FromDomain),
            "to.address" | "to" => Ok(Field::ToAddress),
            "to.domain" => Ok(Field::ToDomain),
            "cc.address" | "cc" => Ok(Field::CarbonCopyAddress),
            "cc.domain" => Ok(Field::CarbonCopyDomain),
            "subject" => Ok(Field::Subject),
            "text" => Ok(Field::Text),
            "html" => Ok(Field::MarkupBody),
            "content" => Ok(Field::Content),
            "message_id" => Ok(Field::MessageIdentifier),
            "received_at" => Ok(Field::ReceivedAt),
            "size" => Ok(Field::Size),
            "attachment_count" => Ok(Field::AttachmentCount),
            "has_attachments" => Ok(Field::HasAttachments),
            _ => Err(QueryError::at_position(
                first_token.source_position,
                "unknown field",
            )),
        }
    }
    /// Check whether the next token is a particular case-normalized keyword.
    fn next_word_is(&self, expected_word: &str) -> bool {
        self.peek_token().is_some_and(
            |token| matches!(&token.kind, TokenKind::Word(word) if word == expected_word),
        )
    }
}

/// Check that a field, operator, and literal use a supported combination.
fn valid_comparison(field: &Field, operator: Operator, value: &Value) -> bool {
    match (field, value, operator) {
        (
            Field::ReceivedAt | Field::Size | Field::AttachmentCount,
            Value::Number(_),
            Operator::Equal
            | Operator::NotEqual
            | Operator::GreaterThan
            | Operator::GreaterThanOrEqual
            | Operator::LessThan
            | Operator::LessThanOrEqual,
        ) => true,
        (Field::HasAttachments, Value::Boolean(_), Operator::Equal | Operator::NotEqual) => true,
        (
            _,
            Value::String(_),
            Operator::Equal
            | Operator::NotEqual
            | Operator::Contains
            | Operator::StartsWith
            | Operator::EndsWith
            | Operator::Matches,
        ) => !matches!(
            field,
            Field::ReceivedAt | Field::Size | Field::AttachmentCount | Field::HasAttachments
        ),
        (
            _,
            Value::String(_),
            Operator::GreaterThan
            | Operator::GreaterThanOrEqual
            | Operator::LessThan
            | Operator::LessThanOrEqual,
        ) => {
            matches!(
                field,
                Field::Subject
                    | Field::Text
                    | Field::MarkupBody
                    | Field::Content
                    | Field::MessageIdentifier
                    | Field::Header(_)
            )
        }
        _ => false,
    }
}
