//! Source locations and script diagnostics with line and column positions.

/// A range of script source with the 1-based line and column at which it starts.
///
/// `start` and `end` are UTF-8 byte offsets; `column` counts Unicode scalar values from the start
/// of the line, so it matches what an editor displays for most text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: u32,
    pub column: u32,
}

/// The first problem that prevents a script from compiling.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}, column {column}: {message}")]
pub struct ScriptError {
    /// A human-readable explanation of the problem and, where possible, how to fix it.
    pub message: String,
    /// Zero-based UTF-8 byte offset of the problem in the source.
    pub position: usize,
    /// 1-based line of the problem.
    pub line: u32,
    /// 1-based column of the problem, in Unicode scalar values.
    pub column: u32,
}

/// A diagnostic located only by byte offset, converted to a [`ScriptError`] at the boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Diagnostic {
    pub(crate) message: String,
    pub(crate) position: usize,
}

impl Diagnostic {
    /**
     * Build a diagnostic at a byte offset.
     */
    pub(crate) fn at(position: usize, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            position,
        }
    }
}

/// Converts byte offsets of one source into line and column positions.
pub(crate) struct SourceMap<'source> {
    source: &'source str,
    line_starts: Vec<usize>,
}

impl<'source> SourceMap<'source> {
    /**
     * Index the start of every line of `source`.
     */
    pub(crate) fn new(source: &'source str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(
            source
                .bytes()
                .enumerate()
                .filter(|(_, byte)| *byte == b'\n')
                .map(|(index, _)| index + 1),
        );
        Self {
            source,
            line_starts,
        }
    }

    /**
     * Return the 1-based line and character column of a byte offset.
     *
     * Offsets beyond the source, or inside a multi-byte character, are clamped to the nearest
     * preceding character boundary.
     */
    pub(crate) fn location(&self, position: usize) -> (u32, u32) {
        let mut position = position.min(self.source.len());
        while !self.source.is_char_boundary(position) {
            position -= 1;
        }
        let line_index = match self.line_starts.binary_search(&position) {
            Ok(index) => index,
            Err(index) => index - 1,
        };
        let line_start = self.line_starts[line_index];
        let column = self.source[line_start..position].chars().count() + 1;
        (
            u32::try_from(line_index + 1).unwrap_or(u32::MAX),
            u32::try_from(column).unwrap_or(u32::MAX),
        )
    }

    /**
     * Build a span from byte offsets.
     */
    pub(crate) fn span(&self, start: usize, end: usize) -> Span {
        let (line, column) = self.location(start);
        Span {
            start,
            end,
            line,
            column,
        }
    }

    /**
     * Convert a byte-located diagnostic into a public error with its line and column.
     */
    pub(crate) fn error(&self, diagnostic: Diagnostic) -> ScriptError {
        let (line, column) = self.location(diagnostic.position);
        ScriptError {
            message: diagnostic.message,
            position: diagnostic.position,
            line,
            column,
        }
    }
}
