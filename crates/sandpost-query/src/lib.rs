mod canonical;
mod compiler;
mod evaluation;
mod expression;
mod glob;
mod lexer;
mod parser;

pub use compiler::{CompiledQuery, EmbeddedQuery, QueryError, compile, compile_embedded};
pub use expression::{Expression, Field, Operator, Predicate, Value};
pub use glob::glob_matches;

#[cfg(test)]
mod tests;
