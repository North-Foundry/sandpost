//! PersonaScript: the small, deterministic language that describes how a Sandpost Persona
//! interacts with one incoming email.
//!
//! A script is a sequence of semicolon-terminated statements with JavaScript-like blocks:
//!
//! ```text
//! wait 5s;
//! open;
//! if (subject contains "Verify") {
//!     wait 2s;
//!     click link("a.verify");
//! } else if (from.domain == "example.com") {
//!     click link(text: "Confirm*");
//! } else {
//!     click link(href: "*unsubscribe*");
//! }
//! ```
//!
//! Source is tokenized by a dedicated lexer and parsed into a typed syntax tree
//! ([`Script`]). Conditions are not a second expression language: the text between `if (` and its
//! closing parenthesis is compiled by `sandpost-query`, so conditions have exactly the fields,
//! operators, case rules, and diagnostics of Sandpost queries. [`compile`] then lowers the tree to
//! a flat [`Program`] of instructions with forward jumps, which [`execute`] runs sequentially
//! against one message's facts. Actions (`open`, `wait`, `click`) are delegated to an
//! [`ActionHandler`] supplied by the caller, so this crate performs no I/O, owns no clock and has
//! no knowledge of storage, HTTP servers, or SMTP. [`document`] holds the I/O-free HTML helpers an
//! action handler uses to find remote images and to select the link a `click` targets.
//!
//! The language deliberately has no variables and no loops. Every block is entered at most once
//! and every jump moves forward, so a program performs at most one action per instruction.

mod diagnostics;
pub mod document;
mod interpreter;
mod lexer;
mod parser;
mod program;
mod syntax;

pub use diagnostics::{ScriptError, Span};
pub use interpreter::{ActionHandler, ExecutionReport, Step, execute};
pub use parser::parse;
pub use program::{Instruction, Operation, Program, compile};
pub use syntax::{
    Alternative, Condition, Conditional, CssSelector, LinkSelector, Script, ScriptLimits,
    Statement, StatementKind,
};

#[cfg(test)]
mod tests;
