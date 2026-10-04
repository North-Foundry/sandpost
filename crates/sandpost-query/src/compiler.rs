//! Compilation entry point, diagnostics, and immutable compiled queries.
use crate::{
    canonical::fingerprint, expression::Expression, lexer::tokenize, parser::parse_tokens,
};
use sandpost_core::MessageFacts;

const MAXIMUM_SOURCE_LENGTH: usize = 16 * 1024;

/// An immutable canonical query and the fingerprint of that exact expression.
///
/// ```compile_fail
/// use sandpost_query::{compile, Expression};
/// let mut query = compile("true").unwrap();
/// query.expression = Expression::False;
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledQuery {
    expression: Expression,
    fingerprint: String,
}

impl CompiledQuery {
    /// Borrow the canonical expression without invalidating its fingerprint.
    pub fn expression(&self) -> &Expression {
        &self.expression
    }

    /// Consume the compiled query to take ownership of its expression.
    pub fn into_expression(self) -> Expression {
        self.expression
    }

    /// SHA-256 of the canonical expression's versioned v1 encoding.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Evaluate the canonical query against one message's facts.
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
    /// Build a diagnostic at a zero-based UTF-8 byte position.
    pub(crate) fn at_position(position: usize, message: impl Into<String>) -> Self {
        Self {
            position,
            message: message.into(),
        }
    }
}

/// Compile and validate a filter; empty input accepts every message.
pub fn compile(source: &str) -> Result<CompiledQuery, QueryError> {
    if source.len() > MAXIMUM_SOURCE_LENGTH {
        return Err(QueryError::at_position(
            MAXIMUM_SOURCE_LENGTH,
            "query exceeds maximum source length",
        ));
    }
    if source.trim().is_empty() {
        return Ok(build_compiled_query(Expression::True));
    }
    let expression = parse_tokens(tokenize(source)?)?.canonicalize();
    Ok(build_compiled_query(expression))
}

/// Pair a canonical expression with its versioned fingerprint.
fn build_compiled_query(expression: Expression) -> CompiledQuery {
    let fingerprint = fingerprint(&expression);
    CompiledQuery {
        expression,
        fingerprint,
    }
}
