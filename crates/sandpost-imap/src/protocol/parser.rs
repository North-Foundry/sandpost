//! Recursive-descent parser for complete client commands (RFC 3501 section 9 grammar).
//!
//! The session hands the parser one complete command: every line and literal, with literal
//! octets inline after their `{n}` CRLF announcement exactly as they arrived, and without the
//! final CRLF. Parsing never allocates more than the input size and bounds recursion.

mod commands;
mod cursor;
mod fetch;
mod search;
#[cfg(test)]
mod tests;

use crate::protocol::command::Command;
use cursor::Cursor;

/// A command that could not be parsed, answered with `BAD`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// The command tag when it could be read; untagged `* BAD` otherwise.
    pub tag: Option<String>,
    /// Human-readable explanation sent as response text.
    pub message: String,
}

/// Parse one complete command.
pub fn parse_command(input: &[u8]) -> Result<Command, ParseError> {
    let mut cursor = Cursor::new(input);
    let tag = cursor
        .tag()
        .map_err(|message| ParseError { tag: None, message })?;
    let kind = cursor.command_after_tag().map_err(|message| ParseError {
        tag: Some(tag.clone()),
        message,
    })?;
    Ok(Command { tag, kind })
}

/// Read the tag and command name of a possibly incomplete command, for decisions taken before
/// the rest of it arrives (such as refusing an APPEND literal).
pub fn command_prefix(input: &[u8]) -> Option<(String, String)> {
    let mut cursor = Cursor::new(input);
    let tag = cursor.tag().ok()?;
    cursor.space().ok()?;
    let mut name = cursor.atom().ok()?.to_ascii_uppercase();
    if name == "UID" {
        cursor.space().ok()?;
        name = format!("UID {}", cursor.atom().ok()?.to_ascii_uppercase());
    }
    Some((tag, name))
}

/// Read the first mailbox argument of a partial APPEND command, if it is complete.
pub fn append_mailbox_prefix(input: &[u8]) -> Option<String> {
    let mut cursor = Cursor::new(input);
    cursor.tag().ok()?;
    cursor.space().ok()?;
    cursor.atom().ok()?;
    cursor.space().ok()?;
    cursor.mailbox().ok()
}
