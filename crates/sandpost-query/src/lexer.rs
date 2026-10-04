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

/// Convert query source into typed tokens with UTF-8 byte positions.
pub(crate) fn tokenize(source: &str) -> Result<Vec<Token>, QueryError> {
    let bytes = source.as_bytes();
    let mut byte_index = 0;
    let mut tokens = Vec::new();
    while byte_index < bytes.len() {
        if bytes[byte_index].is_ascii_whitespace() {
            byte_index += 1;
            continue;
        }
        let source_position = byte_index;
        let single_character_kind = match bytes[byte_index] {
            b'(' => Some(TokenKind::LeftParenthesis),
            b')' => Some(TokenKind::RightParenthesis),
            b'[' => Some(TokenKind::LeftBracket),
            b']' => Some(TokenKind::RightBracket),
            b'.' => Some(TokenKind::Dot),
            b'=' if bytes.get(byte_index + 1) == Some(&b'=') => {
                byte_index += 1;
                Some(TokenKind::Equal)
            }
            b'!' if bytes.get(byte_index + 1) == Some(&b'=') => {
                byte_index += 1;
                Some(TokenKind::NotEqual)
            }
            b'>' if bytes.get(byte_index + 1) == Some(&b'=') => {
                byte_index += 1;
                Some(TokenKind::GreaterThanOrEqual)
            }
            b'>' => Some(TokenKind::GreaterThan),
            b'<' if bytes.get(byte_index + 1) == Some(&b'=') => {
                byte_index += 1;
                Some(TokenKind::LessThanOrEqual)
            }
            b'<' => Some(TokenKind::LessThan),
            _ => None,
        };
        let kind = if let Some(simple_kind) = single_character_kind {
            byte_index += 1;
            simple_kind
        } else if bytes[byte_index] == b'"' || bytes[byte_index] == b'\'' {
            let quote = bytes[byte_index];
            byte_index += 1;
            let mut value = String::new();
            while byte_index < bytes.len() && bytes[byte_index] != quote {
                if bytes[byte_index] == b'\\' {
                    byte_index += 1;
                    if byte_index >= bytes.len() {
                        return Err(QueryError::at_position(
                            source_position,
                            "unterminated escape",
                        ));
                    }
                    let escaped_character = source[byte_index..].chars().next().unwrap();
                    value.push(match escaped_character {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        other => other,
                    });
                    byte_index += escaped_character.len_utf8();
                } else {
                    let character = source[byte_index..].chars().next().unwrap();
                    value.push(character);
                    byte_index += character.len_utf8();
                }
            }
            if byte_index == bytes.len() {
                return Err(QueryError::at_position(
                    source_position,
                    "unterminated string",
                ));
            }
            byte_index += 1;
            TokenKind::String(value)
        } else if bytes[byte_index].is_ascii_digit()
            || (bytes[byte_index] == b'-'
                && bytes.get(byte_index + 1).is_some_and(u8::is_ascii_digit))
        {
            byte_index += 1;
            while byte_index < bytes.len() && bytes[byte_index].is_ascii_digit() {
                byte_index += 1;
            }
            let number = source[source_position..byte_index].parse().map_err(|_| {
                QueryError::at_position(source_position, "number is outside the supported range")
            })?;
            TokenKind::Number(number)
        } else if bytes[byte_index].is_ascii_alphabetic() || bytes[byte_index] == b'_' {
            byte_index += 1;
            while byte_index < bytes.len()
                && (bytes[byte_index].is_ascii_alphanumeric()
                    || matches!(bytes[byte_index], b'_' | b'-'))
            {
                byte_index += 1;
            }
            TokenKind::Word(source[source_position..byte_index].to_ascii_lowercase())
        } else {
            return Err(QueryError::at_position(byte_index, "unexpected character"));
        };
        tokens.push(Token {
            kind,
            source_position,
        });
        if tokens.len() > MAXIMUM_TOKEN_COUNT {
            return Err(QueryError::at_position(
                source_position,
                "query exceeds maximum token count",
            ));
        }
    }
    tokens.push(Token {
        kind: TokenKind::End,
        source_position: source.len(),
    });
    Ok(tokens)
}
