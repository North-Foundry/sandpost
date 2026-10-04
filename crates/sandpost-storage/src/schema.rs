//! Validation of the current SQLite schema baseline.
use crate::{StorageError, migrations::BASELINE_MIGRATIONS};
use rusqlite::Connection;

type SchemaObject = (String, String, String, String);

/// Read all user-defined objects, preserving constraints and index definitions in their SQL.
fn schema_objects(connection: &Connection) -> Result<Vec<SchemaObject>, StorageError> {
    let mut statement = connection.prepare(
        "SELECT type, name, tbl_name, sql FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' ORDER BY type, name",
    )?;
    let objects = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                normalized_definition(&row.get::<_, String>(3)?),
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(objects)
}

/// Compare SQL tokens without discarding word boundaries or changing quoted values.
fn normalized_definition(definition: &str) -> String {
    let mut characters = definition.chars().peekable();
    let mut tokens = Vec::new();
    while let Some(character) = characters.next() {
        if character.is_ascii_whitespace() {
            continue;
        }
        let mut token = String::from(character);
        let delimiter = match character {
            '\'' | '"' | '`' => Some(character),
            '[' => Some(']'),
            _ => None,
        };
        if let Some(delimiter) = delimiter {
            while let Some(quoted_character) = characters.next() {
                token.push(quoted_character);
                if quoted_character == delimiter {
                    if delimiter != ']' && characters.peek() == Some(&delimiter) {
                        token.push(characters.next().expect("peeked delimiter"));
                    } else {
                        break;
                    }
                }
            }
        } else if character.is_alphanumeric() || character == '_' {
            while characters
                .peek()
                .is_some_and(|next| next.is_alphanumeric() || *next == '_')
            {
                token.push(characters.next().expect("peeked word character"));
            }
        }
        tokens.push(token);
    }
    tokens.join(" ")
}

/// Construct the immutable expected schema using the same checked-in table migrations.
fn expected_schema() -> Result<Connection, StorageError> {
    let connection = Connection::open_in_memory()?;
    for (_, script) in BASELINE_MIGRATIONS {
        connection.execute_batch(script)?;
    }
    Ok(connection)
}

/// Detect a fresh database without accepting unrelated application objects.
pub(crate) fn schema_is_empty(connection: &Connection) -> Result<bool, StorageError> {
    Ok(schema_objects(connection)?.is_empty())
}

/// Verify every table, constraint, index, view, and trigger against the current baseline.
pub(crate) fn validate_current_schema(connection: &Connection) -> Result<(), StorageError> {
    if schema_objects(connection)? != schema_objects(&expected_schema()?)? {
        return Err(StorageError::InvalidData(
            "database schema does not match baseline 1".into(),
        ));
    }
    Ok(())
}
