#![doc = include_str!("../README.md")]

mod canonical;
mod compiler;
mod evaluation;
mod expression;
mod glob;
mod lexer;
mod parser;

pub use compiler::{CompiledQuery, QueryError, compile};
pub use expression::{Expression, Field, Operator, Predicate, Value};

#[cfg(test)]
mod tests;
