//! Canonical user persistence on SQLite.
use crate::SqliteStorage;
use crate::error::StorageResult;
use crate::records::invalid_column;
use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sandpost_core::{GlobalRole, MailAccess, User, UserIdentifier};
use sandpost_storage::{NewUser, StorageError, UpdateUser, UserStorage};

const USER_COLUMNS: &str = "identifier,name,email,password_hash,global_role,personal_filter,created_at,updated_at,mail_access";

/// Return the stored text for a global role.
pub(crate) fn global_role_text(role: GlobalRole) -> &'static str {
    match role {
        GlobalRole::Owner => "owner",
        GlobalRole::Admin => "admin",
        GlobalRole::Member => "member",
        GlobalRole::Viewer => "viewer",
    }
}

/// Return the stored text for a mail access mode.
fn mail_access_text(access: MailAccess) -> &'static str {
    match access {
        MailAccess::All => "all",
        MailAccess::Scoped => "scoped",
    }
}

/// Parse a stored mail access mode, rejecting unknown values.
fn parse_mail_access(value: &str) -> Option<MailAccess> {
    match value {
        "all" => Some(MailAccess::All),
        "scoped" => Some(MailAccess::Scoped),
        _ => None,
    }
}

/// Parse a stored global role, rejecting unknown values.
fn parse_global_role(value: &str) -> Option<GlobalRole> {
    match value {
        "owner" => Some(GlobalRole::Owner),
        "admin" => Some(GlobalRole::Admin),
        "member" => Some(GlobalRole::Member),
        "viewer" => Some(GlobalRole::Viewer),
        _ => None,
    }
}

/// Current Unix time in seconds for created/updated timestamps.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// Decode one canonical user row in the fixed selection order.
fn decode_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    let identifier: String = row.get(0)?;
    let role: String = row.get(4)?;
    let access: String = row.get(8)?;
    Ok(User {
        identifier: identifier
            .parse()
            .map_err(|_| invalid_column(0, "identifier"))?,
        name: row.get(1)?,
        email: row.get(2)?,
        password_hash: row.get(3)?,
        global_role: parse_global_role(&role).ok_or_else(|| invalid_column(4, "global_role"))?,
        mail_access: parse_mail_access(&access).ok_or_else(|| invalid_column(8, "mail_access"))?,
        personal_filter: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

/// Read one user by an internal key column and bound value.
fn read_user(
    connection: &Connection,
    column: &str,
    value: &str,
) -> Result<Option<User>, StorageError> {
    let query = format!("SELECT {USER_COLUMNS} FROM users WHERE {column}=?1");
    connection
        .query_row(&query, [value], decode_user)
        .optional()
        .storage()
}

/// Read one user by identifier within the caller's transaction.
fn read_user_by_identifier(
    connection: &Connection,
    identifier: UserIdentifier,
) -> Result<Option<User>, StorageError> {
    read_user(connection, "identifier", &identifier.to_string())
}

/// Insert one user row, assuming duplicate email has already been rejected.
fn insert_user(connection: &Connection, user: &NewUser) -> Result<User, StorageError> {
    let identifier = UserIdentifier::new();
    let now = now_unix();
    connection
        .execute(
            "INSERT INTO users(identifier,name,email,password_hash,global_role,personal_filter,created_at,updated_at,mail_access) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                identifier.to_string(),
                user.name,
                user.email,
                user.password_hash,
                global_role_text(user.global_role),
                user.personal_filter,
                now,
                now,
                mail_access_text(user.mail_access),
            ],
        )
        .storage()?;
    Ok(User {
        identifier,
        name: user.name.clone(),
        email: user.email.clone(),
        password_hash: user.password_hash.clone(),
        global_role: user.global_role,
        mail_access: user.mail_access,
        personal_filter: user.personal_filter.clone(),
        created_at: now,
        updated_at: now,
    })
}

/// Count users with the owner role in the caller's transaction.
fn owner_count(connection: &Connection) -> Result<i64, StorageError> {
    connection
        .query_row(
            "SELECT COUNT(*) FROM users WHERE global_role='owner'",
            [],
            |row| row.get(0),
        )
        .storage()
}

/// Create a user with transactional protection against duplicate email addresses.
fn create_user_blocking(connection: &mut Connection, user: &NewUser) -> Result<User, StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    let duplicate: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE email=?1)",
            [&user.email],
            |row| row.get(0),
        )
        .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateUserEmail);
    }
    let created = insert_user(&transaction, user)?;
    transaction.commit().storage()?;
    Ok(created)
}

/// Create the first user as an Owner with All mail access, refusing repeated setup.
fn create_first_owner_blocking(
    connection: &mut Connection,
    owner: &NewUser,
) -> Result<User, StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    let has_users: bool = transaction
        .query_row("SELECT EXISTS(SELECT 1 FROM users)", [], |row| row.get(0))
        .storage()?;
    if has_users {
        return Err(StorageError::InstanceAlreadyInitialized);
    }
    let owner = NewUser {
        global_role: GlobalRole::Owner,
        mail_access: MailAccess::All,
        ..owner.clone()
    };
    let created = insert_user(&transaction, &owner)?;
    transaction.commit().storage()?;
    Ok(created)
}

/// Update a user atomically while preserving email uniqueness and the final Owner.
fn update_user_blocking(
    connection: &mut Connection,
    identifier: UserIdentifier,
    changes: &UpdateUser,
) -> Result<Option<User>, StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    let Some(current) = read_user_by_identifier(&transaction, identifier)? else {
        return Ok(None);
    };
    if current.global_role == GlobalRole::Owner
        && changes.global_role != GlobalRole::Owner
        && owner_count(&transaction)? <= 1
    {
        return Err(StorageError::LastOwner);
    }
    let duplicate: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM users WHERE email=?1 AND identifier<>?2)",
            params![changes.email, identifier.to_string()],
            |row| row.get(0),
        )
        .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateUserEmail);
    }
    transaction
        .execute(
            "UPDATE users SET name=?2,email=?3,password_hash=?4,global_role=?5,personal_filter=?6,updated_at=?7,mail_access=?8 WHERE identifier=?1",
            params![
                identifier.to_string(),
                changes.name,
                changes.email,
                changes.password_hash,
                global_role_text(changes.global_role),
                changes.personal_filter,
                now_unix(),
                mail_access_text(changes.mail_access),
            ],
        )
        .storage()?;
    let updated = read_user_by_identifier(&transaction, identifier)?;
    transaction.commit().storage()?;
    Ok(updated)
}

/// Delete a user and dependent records while preserving the final Owner.
fn delete_user_blocking(
    connection: &mut Connection,
    identifier: UserIdentifier,
) -> Result<bool, StorageError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .storage()?;
    if let Some(current) = read_user_by_identifier(&transaction, identifier)?
        && current.global_role == GlobalRole::Owner
        && owner_count(&transaction)? <= 1
    {
        return Err(StorageError::LastOwner);
    }
    let deleted = transaction
        .execute(
            "DELETE FROM users WHERE identifier=?1",
            [identifier.to_string()],
        )
        .storage()?
        > 0;
    transaction.commit().storage()?;
    Ok(deleted)
}

#[async_trait]
impl UserStorage for SqliteStorage {
    /// Read a canonical user by identifier.
    async fn get_user(&self, identifier: UserIdentifier) -> Result<Option<User>, StorageError> {
        self.run(move |connection| read_user(connection, "identifier", &identifier.to_string()))
            .await
    }

    /// Read a canonical user by its unique email address.
    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, StorageError> {
        let email = email.to_owned();
        self.run(move |connection| read_user(connection, "email", &email))
            .await
    }

    /// List canonical users in deterministic identifier order.
    async fn list_users(&self) -> Result<Vec<User>, StorageError> {
        self.run(|connection| {
            let query = format!("SELECT {USER_COLUMNS} FROM users ORDER BY identifier");
            let mut statement = connection.prepare(&query).storage()?;
            statement
                .query_map([], decode_user)
                .storage()?
                .collect::<Result<Vec<_>, _>>()
                .storage()
        })
        .await
    }

    /// Create a user while preserving email uniqueness across concurrent callers.
    async fn create_user(&self, user: &NewUser) -> Result<User, StorageError> {
        let user = user.clone();
        self.run(move |connection| create_user_blocking(connection, &user))
            .await
    }

    /// Update a user without allowing deletion of the last Owner authority.
    async fn update_user(
        &self,
        identifier: UserIdentifier,
        changes: &UpdateUser,
    ) -> Result<Option<User>, StorageError> {
        let changes = changes.clone();
        self.run(move |connection| update_user_blocking(connection, identifier, &changes))
            .await
    }

    /// Delete a user with cascading owned records, refusing the final Owner.
    async fn delete_user(&self, identifier: UserIdentifier) -> Result<bool, StorageError> {
        self.run(move |connection| delete_user_blocking(connection, identifier))
            .await
    }

    /// Count canonical users for first-run setup and health decisions.
    async fn count_users(&self) -> Result<u64, StorageError> {
        self.run(|connection| {
            let count: i64 = connection
                .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
                .storage()?;
            u64::try_from(count).map_err(|_| StorageError::IntegerRange)
        })
        .await
    }

    /// Atomically bootstrap the installation with its first Owner.
    async fn create_first_owner(&self, owner: &NewUser) -> Result<User, StorageError> {
        let owner = owner.clone();
        self.run(move |connection| create_first_owner_blocking(connection, &owner))
            .await
    }
}
