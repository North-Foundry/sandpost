//! Bounded tokenization with UTF-8 byte positions.
use crate::compiler::QueryError;

const MAXIMUM_TOKEN_COUNT: usize = 4096;

#[derive(Debug, Clone)]
pub(crate) struct Token {
    pub(crate) kind: TokenKind,
    pub(crate) source_position: usize,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TokenKind {
    Word(String),
    String(String),
    Number(i64),
    LeftParenthesis,
    RightParenthesis,
    LeftBracket,
    RightBracket,
    Dot,
    Equal,
    NotEqual,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    End,
}

/**
 * Convert query source into typed tokens with UTF-8 byte positions.
 */
pub(crate) fn tokenize(source: &str) -> Result<Vec<Token>, QueryError> {
    let mut byte_index = 0;
    let mut tokens = Vec::new();
    while let Some(token) = next_token(source, &mut byte_index)? {
        push_bounded(&mut tokens, token)?;
    }
    tokens.push(Token {
        kind: TokenKind::End,
        source_position: source.len(),
    });
    Ok(tokens)
}

/**
 * Tokenize a query embedded in a host language, starting just after the host's opening `(`.
 *
 * Tokenization stops at the first right parenthesis that is not balanced by a left parenthesis
 * of the query itself; parentheses inside string literals are ordinary string content because
 * they are consumed by the same string scanner as [`tokenize`]. The returned tokens end with a
 * sentinel positioned at that closing parenthesis, whose byte index is returned alongside them.
 * Positions remain absolute byte offsets in the complete host source.
 */
pub(crate) fn tokenize_embedded(
    source: &str,
    start: usize,
) -> Result<(Vec<Token>, usize), QueryError> {
    if start > source.len() || !source.is_char_boundary(start) {
        return Err(QueryError::at_position(
            start,
            "invalid embedded query start",
        ));
    }
    let mut byte_index = start;
    let mut tokens = Vec::new();
    let mut parenthesis_depth = 0usize;
    while let Some(token) = next_token(source, &mut byte_index)? {
        match token.kind {
            TokenKind::LeftParenthesis => parenthesis_depth += 1,
            TokenKind::RightParenthesis if parenthesis_depth == 0 => {
                let end = token.source_position;
                tokens.push(Token {
                    kind: TokenKind::End,
                    source_position: end,
                });
                return Ok((tokens, end));
            }
            TokenKind::RightParenthesis => parenthesis_depth -= 1,
            _ => {}
        }
        push_bounded(&mut tokens, token)?;
    }
    Err(QueryError::at_position(
        source.len(),
        "expected ')' after the condition",
    ))
}

/**
 * Append a token while enforcing the maximum token count shared by every tokenization mode.
 */
fn push_bounded(tokens: &mut Vec<Token>, token: Token) -> Result<(), QueryError> {
    let source_position = token.source_position;
    tokens.push(token);
    if tokens.len() > MAXIMUM_TOKEN_COUNT {
        return Err(QueryError::at_position(
            source_position,
            "query exceeds maximum token count",
        ));
    }
    Ok(())
}

/**
 * Scan the next token at or after `byte_index`, skipping ASCII whitespace.
 *
 * Returns `None` at the end of the source and leaves `byte_index` just after the token.
 */
fn next_token(source: &str, byte_index: &mut usize) -> Result<Option<Token>, QueryError> {
    let bytes = source.as_bytes();
    while *byte_index < bytes.len() && bytes[*byte_index].is_ascii_whitespace() {
        *byte_index += 1;
    }
    if *byte_index >= bytes.len() {
        return Ok(None);
    }
    let mut position = *byte_index;
    let source_position = position;
    let single_character_kind = match bytes[position] {
        b'(' => Some(TokenKind::LeftParenthesis),
        b')' => Some(TokenKind::RightParenthesis),
        b'[' => Some(TokenKind::LeftBracket),
        b']' => Some(TokenKind::RightBracket),
        b'.' => Some(TokenKind::Dot),
        b'=' if bytes.get(position + 1) == Some(&b'=') => {
            position += 1;
            Some(TokenKind::Equal)
        }
        b'!' if bytes.get(position + 1) == Some(&b'=') => {
            position += 1;
            Some(TokenKind::NotEqual)
        }
        b'>' if bytes.get(position + 1) == Some(&b'=') => {
            position += 1;
            Some(TokenKind::GreaterThanOrEqual)
        }
        b'>' => Some(TokenKind::GreaterThan),
        b'<' if bytes.get(position + 1) == Some(&b'=') => {
            position += 1;
            Some(TokenKind::LessThanOrEqual)
        }
        b'<' => Some(TokenKind::LessThan),
        _ => None,
    };
    let kind = if let Some(simple_kind) = single_character_kind {
        position += 1;
        simple_kind
    } else if bytes[position] == b'"' || bytes[position] == b'\'' {
        let quote = bytes[position];
        position += 1;
        let mut value = String::new();
        while position < bytes.len() && bytes[position] != quote {
            if bytes[position] == b'\\' {
                position += 1;
                if position >= bytes.len() {
                    return Err(QueryError::at_position(
                        source_position,
                        "unterminated escape",
                    ));
                }
                let escaped_character = source[position..].chars().next().unwrap();
                value.push(match escaped_character {
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    other => other,
                });
                position += escaped_character.len_utf8();
            } else {
                let character = source[position..].chars().next().unwrap();
                value.push(character);
                position += character.len_utf8();
            }
        }
        if position == bytes.len() {
            return Err(QueryError::at_position(
                source_position,
                "unterminated string",
            ));
        }
        position += 1;
        TokenKind::String(value)
    } else if bytes[position].is_ascii_digit()
        || (bytes[position] == b'-' && bytes.get(position + 1).is_some_and(u8::is_ascii_digit))
    {
        position += 1;
        while position < bytes.len() && bytes[position].is_ascii_digit() {
            position += 1;
        }
        let number = source[source_position..position].parse().map_err(|_| {
            QueryError::at_position(source_position, "number is outside the supported range")
        })?;
        TokenKind::Number(number)
    } else if bytes[position].is_ascii_alphabetic() || bytes[position] == b'_' {
        position += 1;
        while position < bytes.len()
            && (bytes[position].is_ascii_alphanumeric() || matches!(bytes[position], b'_' | b'-'))
        {
            position += 1;
        }
        TokenKind::Word(source[source_position..position].to_ascii_lowercase())
    } else {
        return Err(QueryError::at_position(position, "unexpected character"));
    };
    *byte_index = position;
    Ok(Some(Token {
        kind,
        source_position,
    }))
}
