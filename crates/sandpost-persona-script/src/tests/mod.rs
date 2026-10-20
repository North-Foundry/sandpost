//! Unit tests grouped by responsibility.
use crate::{ScriptError, ScriptLimits, compile, parse};

mod document;
mod interpreter;
mod parsing;
mod program;

/**
 * Parse with default limits and return the error, failing the test if parsing succeeds.
 */
fn parse_error(source: &str) -> ScriptError {
    match parse(source, &ScriptLimits::default()) {
        Ok(script) => panic!("expected an error for {source:?}, parsed {script:?}"),
        Err(error) => error,
    }
}

/**
 * Compile with default limits, failing the test with the diagnostic on error.
 */
fn compiled(source: &str) -> crate::Program {
    compile(source, &ScriptLimits::default()).unwrap_or_else(|error| panic!("{error}"))
}
