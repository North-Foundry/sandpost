//! Canonical user persistence on SQLite.
use crate::SqliteStorage;
use crate::error::StorageResult;
use crate::records::invalid_column;
use async_trait::async_trait;
use sandpost_core::{GlobalRole, MailAccess, User, UserIdentifier};
use sandpost_storage::{NewUser, StorageError, UpdateUser, UserStorage};
use sqlx::{Row, SqliteConnection, sqlite::SqliteRow};

const USER_COLUMNS: &str = "identifier,name,email,password_hash,global_role,personal_filter,created_at,updated_at,mail_access";

/// Return the stored text for a global role.
pub(crate) fn global_role_text(role: GlobalRole) -> &'static str {
    match role {
        GlobalRole::Owner => "owner",
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
        "member" => Some(GlobalRole::Member),
        "viewer" => Some(GlobalRole::Viewer),
        _ => None,
    }
}

/// Current Unix time in seconds for created and updated timestamps.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// Decode one canonical user row by its selected column names.
fn decode_user(row: SqliteRow) -> Result<User, sqlx::Error> {
    let identifier: String = row.try_get("identifier")?;
    let role: String = row.try_get("global_role")?;
    let access: String = row.try_get("mail_access")?;
    Ok(User {
        identifier: identifier
            .parse()
            .map_err(|_| invalid_column("identifier"))?,
        name: row.try_get("name")?,
        email: row.try_get("email")?,
        password_hash: row.try_get("password_hash")?,
        global_role: parse_global_role(&role).ok_or_else(|| invalid_column("global_role"))?,
        mail_access: parse_mail_access(&access).ok_or_else(|| invalid_column("mail_access"))?,
        personal_filter: row.try_get("personal_filter")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

/// Read one user by an internal key column and bound value.
async fn read_user(
    connection: &mut SqliteConnection,
    column: &str,
    value: &str,
) -> Result<Option<User>, StorageError> {
    let query = format!("SELECT {USER_COLUMNS} FROM users WHERE {column}=?1");
    sqlx::query(sqlx::AssertSqlSafe(query))
        .bind(value)
        .fetch_optional(connection)
        .await
        .storage()?
        .map(decode_user)
        .transpose()
        .storage()
}

/// Read one user by identifier within the caller's transaction.
async fn read_user_by_identifier(
    connection: &mut SqliteConnection,
    identifier: UserIdentifier,
) -> Result<Option<User>, StorageError> {
    read_user(connection, "identifier", &identifier.to_string()).await
}

/// Insert one user row, assuming duplicate email has already been rejected.
async fn insert_user(
    connection: &mut SqliteConnection,
    user: &NewUser,
) -> Result<User, StorageError> {
    let identifier = UserIdentifier::new();
    let now = now_unix();
    sqlx::query(
        "INSERT INTO users(identifier,name,email,password_hash,global_role,personal_filter,created_at,updated_at,mail_access) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
    )
    .bind(identifier.to_string())
    .bind(&user.name)
    .bind(&user.email)
    .bind(&user.password_hash)
    .bind(global_role_text(user.global_role))
    .bind(&user.personal_filter)
    .bind(now)
    .bind(now)
    .bind(mail_access_text(user.mail_access))
    .execute(&mut *connection)
    .await
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
async fn owner_count(connection: &mut SqliteConnection) -> Result<i64, StorageError> {
    sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE global_role='owner'")
        .fetch_one(connection)
        .await
        .storage()
}

/// Create a user while preserving email uniqueness across concurrent callers.
async fn create_user(storage: &SqliteStorage, user: &NewUser) -> Result<User, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let duplicate: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE email=?1)")
        .bind(&user.email)
        .fetch_one(&mut *transaction)
        .await
        .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateUserEmail);
    }
    let created = insert_user(&mut transaction, user).await?;
    transaction.commit().await.storage()?;
    Ok(created)
}

/// Create an Owner with All mail access only when no Owner exists, preserving other users.
async fn create_first_owner(
    storage: &SqliteStorage,
    owner: &NewUser,
) -> Result<User, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    if owner_count(&mut transaction).await? > 0 {
        return Err(StorageError::InstanceAlreadyInitialized);
    }
    if read_user(&mut transaction, "email", &owner.email)
        .await?
        .is_some()
    {
        return Err(StorageError::DuplicateUserEmail);
    }
    let owner = NewUser {
        global_role: GlobalRole::Owner,
        mail_access: MailAccess::All,
        ..owner.clone()
    };
    let created = insert_user(&mut transaction, &owner).await?;
    transaction.commit().await.storage()?;
    Ok(created)
}

/// Update a user atomically while preserving email uniqueness and the final Owner.
async fn update_user(
    storage: &SqliteStorage,
    identifier: UserIdentifier,
    changes: &UpdateUser,
) -> Result<Option<User>, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    let Some(current) = read_user_by_identifier(&mut transaction, identifier).await? else {
        return Ok(None);
    };
    if current.global_role == GlobalRole::Owner
        && changes.global_role != GlobalRole::Owner
        && owner_count(&mut transaction).await? <= 1
    {
        return Err(StorageError::LastOwner);
    }
    let duplicate: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE email=?1 AND identifier<>?2)")
            .bind(&changes.email)
            .bind(identifier.to_string())
            .fetch_one(&mut *transaction)
            .await
            .storage()?;
    if duplicate {
        return Err(StorageError::DuplicateUserEmail);
    }
    sqlx::query(
        "UPDATE users SET name=?2,email=?3,password_hash=?4,global_role=?5,personal_filter=?6,updated_at=?7,mail_access=?8 WHERE identifier=?1",
    )
    .bind(identifier.to_string())
    .bind(&changes.name)
    .bind(&changes.email)
    .bind(&changes.password_hash)
    .bind(global_role_text(changes.global_role))
    .bind(&changes.personal_filter)
    .bind(now_unix())
    .bind(mail_access_text(changes.mail_access))
    .execute(&mut *transaction)
    .await
    .storage()?;
    let updated = read_user_by_identifier(&mut transaction, identifier).await?;
    transaction.commit().await.storage()?;
    Ok(updated)
}

/// Delete a user and dependent records while preserving the final Owner.
async fn delete_user(
    storage: &SqliteStorage,
    identifier: UserIdentifier,
) -> Result<bool, StorageError> {
    let mut transaction = storage
        .write_pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .storage()?;
    if let Some(current) = read_user_by_identifier(&mut transaction, identifier).await?
        && current.global_role == GlobalRole::Owner
        && owner_count(&mut transaction).await? <= 1
    {
        return Err(StorageError::LastOwner);
    }
    let deleted = sqlx::query("DELETE FROM users WHERE identifier=?1")
        .bind(identifier.to_string())
        .execute(&mut *transaction)
        .await
        .storage()?
        .rows_affected()
        > 0;
    crate::imap::organization::collect_unreferenced_messages(&mut transaction).await?;
    transaction.commit().await.storage()?;
    Ok(deleted)
}

#[async_trait]
impl UserStorage for SqliteStorage {
    /// Read a canonical user by identifier.
    async fn get_user(&self, identifier: UserIdentifier) -> Result<Option<User>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        read_user(&mut connection, "identifier", &identifier.to_string()).await
    }

    /// Read a canonical user by its unique email address.
    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, StorageError> {
        let mut connection = self.read_pool.acquire().await.storage()?;
        read_user(&mut connection, "email", email).await
    }

    /// List canonical users in deterministic identifier order.
    async fn list_users(&self) -> Result<Vec<User>, StorageError> {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SELECT {USER_COLUMNS} FROM users ORDER BY identifier"
        )))
        .fetch_all(&self.read_pool)
        .await
        .storage()?
        .into_iter()
        .map(decode_user)
        .collect::<Result<Vec<_>, _>>()
        .storage()
    }

    /// Create a user while preserving email uniqueness across concurrent callers.
    async fn create_user(&self, user: &NewUser) -> Result<User, StorageError> {
        create_user(self, user).await
    }

    /// Update a user without allowing deletion of the last Owner authority.
    async fn update_user(
        &self,
        identifier: UserIdentifier,
        changes: &UpdateUser,
    ) -> Result<Option<User>, StorageError> {
        update_user(self, identifier, changes).await
    }

    /// Delete a user with cascading owned records, refusing the final Owner.
    async fn delete_user(&self, identifier: UserIdentifier) -> Result<bool, StorageError> {
        delete_user(self, identifier).await
    }

    /// Count canonical user accounts.
    async fn count_users(&self) -> Result<u64, StorageError> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&self.read_pool)
            .await
            .storage()?;
        u64::try_from(count).map_err(|_| StorageError::IntegerRange)
    }

    /// Atomically bootstrap the installation with its first Owner.
    async fn create_first_owner(&self, owner: &NewUser) -> Result<User, StorageError> {
        create_first_owner(self, owner).await
    }

    /**
     * Persist a remembered session keyed by the caller-computed token digest.
     */
    async fn create_persistent_session(
        &self,
        token_hash: &str,
        user_identifier: UserIdentifier,
        verified_password_hash: &str,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO persistent_sessions(token_hash,user_identifier,verified_password_hash) VALUES (?1,?2,?3)",
        )
        .bind(token_hash)
        .bind(user_identifier.to_string())
        .bind(verified_password_hash)
        .execute(&self.write_pool)
        .await
        .storage()?;
        Ok(())
    }

    /**
     * Load a remembered session's user identifier and credential binding.
     */
    async fn get_persistent_session(
        &self,
        token_hash: &str,
    ) -> Result<Option<(UserIdentifier, String)>, StorageError> {
        let row = sqlx::query(
            "SELECT user_identifier,verified_password_hash FROM persistent_sessions WHERE token_hash=?1",
        )
        .bind(token_hash)
        .fetch_optional(&self.read_pool)
        .await
        .storage()?;
        row.map(|row| {
            let identifier: String = row.try_get("user_identifier").storage()?;
            let password_hash: String = row.try_get("verified_password_hash").storage()?;
            let identifier = identifier.parse().map_err(|_| {
                StorageError::InvalidData("invalid persistent session user identifier".into())
            })?;
            Ok((identifier, password_hash))
        })
        .transpose()
    }

    /**
     * Delete a remembered session by token digest, succeeding when it is already absent.
     */
    async fn delete_persistent_session(&self, token_hash: &str) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM persistent_sessions WHERE token_hash=?1")
            .bind(token_hash)
            .execute(&self.write_pool)
            .await
            .storage()?;
        Ok(())
    }

    /**
     * Delete all remembered sessions for one user.
     */
    async fn delete_user_persistent_sessions(
        &self,
        user_identifier: UserIdentifier,
    ) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM persistent_sessions WHERE user_identifier=?1")
            .bind(user_identifier.to_string())
            .execute(&self.write_pool)
            .await
            .storage()?;
        Ok(())
    }
}
