//! Validation of the SQLite physical schema against the checked-in migration scripts.
use crate::{StorageError, error::StorageResult, migrations::batches_through};
use sqlx::{ConnectOptions, Row, SqliteConnection, sqlite::SqliteConnectOptions};

type SchemaObject = (String, String, String, String);

/// Read all user-defined objects, preserving constraints and index definitions in their SQL.
async fn schema_objects(
    connection: &mut SqliteConnection,
) -> Result<Vec<SchemaObject>, StorageError> {
    let objects = sqlx::query(
        "SELECT type, name, tbl_name, sql FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' ORDER BY type, name",
    ).try_map(|row: sqlx::sqlite::SqliteRow| Ok((row.try_get::<String, _>("type")?, row.try_get::<String, _>("name")?, row.try_get::<String, _>("tbl_name")?, row.try_get::<String, _>("sql")?)))
    .fetch_all(connection).await.storage()?;
    Ok(objects
        .into_iter()
        .map(|(kind, name, table, definition)| {
            (kind, name, table, normalized_definition(&definition))
        })
        .collect())
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
            '\'' | '"' | '\x60' => Some(character),
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

/// Construct the immutable expected schema of one version from the checked-in scripts.
async fn expected_schema(version: i64) -> Result<SqliteConnection, StorageError> {
    let mut connection = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true)
        .connect()
        .await
        .storage()?;
    for (_, scripts) in batches_through(version) {
        for (_, script) in scripts.iter() {
            // Use the Send executor future for multi-statement scripts during startup.
            sqlx::Executor::execute(&mut connection, sqlx::raw_sql(*script))
                .await
                .storage()?;
        }
    }
    Ok(connection)
}

/// Detect a fresh database without accepting unrelated application objects.
pub(crate) async fn schema_is_empty(
    connection: &mut SqliteConnection,
) -> Result<bool, StorageError> {
    Ok(schema_objects(connection).await?.is_empty())
}

/// Report whether every table, constraint, index, view, and trigger matches the schema of
/// `version`.
pub(crate) async fn matches_schema(
    connection: &mut SqliteConnection,
    version: i64,
) -> Result<bool, StorageError> {
    let mut expected = expected_schema(version).await?;
    Ok(schema_objects(connection).await? == schema_objects(&mut expected).await?)
}
