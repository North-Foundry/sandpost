//! Installation metadata, ephemeral bootstrap credentials, and shared Owner sessions.
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use sandpost_core::{GlobalRole, MailAccess, User, UserIdentifier};
use sandpost_storage::{NewUser, Storage, StorageError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use uuid::Uuid;

/// Public installation status; never contains credentials or installation internals.
#[derive(Clone, Serialize, Deserialize)]
pub struct InstanceStatus {
    pub initialized: bool,
}

/// Installation identity recorded after the Owner commits; never controls setup availability.
#[derive(Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InstanceMetadata {
    version: u32,
    instance_id: Uuid,
}

/// First-user setup input; intentionally lacks Debug to avoid credential logging.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupRequest {
    /// Legacy transport field accepting the boot key or a temporary setup authorization.
    pub bootstrap_secret: String,
    pub name: String,
    pub email: String,
    pub password: String,
}

/// Normal authentication input shared by HTTP and the native host.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub remember_me: bool,
}

/// Authenticated identity, excluding password hashes and session tokens.
#[derive(Clone, Serialize, Deserialize)]
pub struct SessionUser {
    pub id: UserIdentifier,
    pub name: String,
    pub email: String,
    pub role: GlobalRole,
}

/// Login result; transports decide whether to expose the token or retain it in their host.
#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: SessionUser,
}

/// Failures in installation bootstrap, metadata durability, or authentication.
#[derive(thiserror::Error, Debug)]
pub enum InstanceError {
    #[error("authentication failed")]
    Authentication,
    #[error("initial setup is required")]
    SetupRequired,
    #[error("instance already initialized")]
    AlreadyInitialized,
    #[error("{0}")]
    InvalidInput(String),
    #[error(
        "SandPost instance state is unavailable. Restart to recover its state from the database."
    )]
    RecoveryRequired,
    #[error("Cannot write SandPost instance metadata: {0}")]
    WriteMetadata(std::io::Error),
    #[error("Cannot create SandPost data directory: {0}")]
    CreateDirectory(std::io::Error),
    #[error("storage operation failed: {0}")]
    Storage(#[from] StorageError),
    #[error("instance worker failed: {0}")]
    Worker(#[from] tokio::task::JoinError),
    #[error("cryptographic operation failed: {0}")]
    Cryptography(String),
}

/// Process state derived from database Owners; a failed setup publication locks this process.
enum InstanceState {
    Bootstrap {
        secret: String,
        authorizations: Vec<(String, Instant)>,
    },
    Initialized,
    RecoveryRequired,
}

/// A process-local session with a fixed lifetime; restarting invalidates all sessions.
struct Session {
    user: SessionUser,
    verified_password_hash: String,
    expires: Instant,
}

/// Shared service serializing setup and guarding the sole bootstrap-to-initialized transition.
#[derive(Clone)]
pub struct Instance {
    storage: Arc<dyn Storage>,
    directory: PathBuf,
    state: Arc<Mutex<InstanceState>>,
    /// Serializes async setup so concurrent requests keep the single-winner contract.
    setup_gate: Arc<tokio::sync::Mutex<()>>,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
}

impl Instance {
    /// Open setup only when the database has no Owner, regardless of installation metadata.
    pub async fn open(
        storage: Arc<dyn Storage>,
        data_directory: PathBuf,
    ) -> Result<Self, InstanceError> {
        fs::create_dir_all(&data_directory).map_err(InstanceError::CreateDirectory)?;
        let state = if storage_has_owner(storage.as_ref()).await? {
            InstanceState::Initialized
        } else {
            InstanceState::Bootstrap {
                secret: random_secret("SP")?,
                authorizations: Vec::new(),
            }
        };
        Ok(Self::with_state(storage, data_directory, state))
    }

    /// Fail closed for callers that have not resolved a configured installation at startup.
    pub(crate) fn locked(storage: Arc<dyn Storage>) -> Self {
        Self::with_state(storage, PathBuf::new(), InstanceState::RecoveryRequired)
    }

    fn with_state(storage: Arc<dyn Storage>, directory: PathBuf, state: InstanceState) -> Self {
        Self {
            storage,
            directory,
            state: Arc::new(Mutex::new(state)),
            setup_gate: Arc::new(tokio::sync::Mutex::new(())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Lock installation state without treating poison as an opportunity to run setup.
    fn state(&self) -> Result<MutexGuard<'_, InstanceState>, InstanceError> {
        self.state
            .lock()
            .map_err(|_| InstanceError::RecoveryRequired)
    }

    /// Report the authoritative in-memory mode without disclosing the bootstrap secret.
    pub fn status(&self) -> Result<InstanceStatus, InstanceError> {
        match &*self.state()? {
            InstanceState::Bootstrap { .. } => Ok(InstanceStatus { initialized: false }),
            InstanceState::Initialized => Ok(InstanceStatus { initialized: true }),
            InstanceState::RecoveryRequired => Err(InstanceError::RecoveryRequired),
        }
    }

    /// Supply the key to the startup banner or trusted native host; no transport exposes it.
    pub fn bootstrap_secret_for_startup(&self) -> Result<Option<String>, InstanceError> {
        match &*self.state()? {
            InstanceState::Bootstrap { secret, .. } => Ok(Some(secret.clone())),
            InstanceState::Initialized => Ok(None),
            InstanceState::RecoveryRequired => Err(InstanceError::RecoveryRequired),
        }
    }

    /// Verify only the current boot key without creating an Owner or persisting credentials.
    /// Authorizations expire after ten minutes; at most 64 are retained, evicting the oldest.
    /// Leaving Bootstrap or restarting invalidates every issued authorization.
    pub fn authorize_setup(&self, setup_key: &str) -> Result<String, InstanceError> {
        let mut state = self.state()?;
        let (secret, authorizations) = match &mut *state {
            InstanceState::Bootstrap {
                secret,
                authorizations,
            } => (secret, authorizations),
            InstanceState::Initialized => return Err(InstanceError::AlreadyInitialized),
            InstanceState::RecoveryRequired => return Err(InstanceError::RecoveryRequired),
        };
        if !bool::from(secret.as_bytes().ct_eq(setup_key.as_bytes())) {
            return Err(InstanceError::Authentication);
        }
        let authorization = random_secret("SETUP")?;
        let now = Instant::now();
        authorizations.retain(|(_, expires)| *expires > now);
        if authorizations.len() >= 64 {
            authorizations.remove(0);
        }
        authorizations.push((authorization.clone(), now + Duration::from_secs(600)));
        Ok(authorization)
    }

    /// Run serialized setup, committing the first Owner and publishing installation metadata.
    /// The legacy bootstrapSecret field accepts the boot key or a live setup authorization.
    pub async fn setup(&self, request: SetupRequest) -> Result<InstanceStatus, InstanceError> {
        let _gate = self.setup_gate.lock().await;
        {
            let mut state = self.state()?;
            let (secret, authorizations) = match &mut *state {
                InstanceState::Bootstrap {
                    secret,
                    authorizations,
                } => (secret, authorizations),
                InstanceState::Initialized => return Err(InstanceError::AlreadyInitialized),
                InstanceState::RecoveryRequired => return Err(InstanceError::RecoveryRequired),
            };
            let now = Instant::now();
            authorizations.retain(|(_, expires)| *expires > now);
            let authorized = authorizations.iter().fold(
                bool::from(secret.as_bytes().ct_eq(request.bootstrap_secret.as_bytes())),
                |authorized, (authorization, _)| {
                    authorized
                        | bool::from(
                            authorization
                                .as_bytes()
                                .ct_eq(request.bootstrap_secret.as_bytes()),
                        )
                },
            );
            if !authorized {
                return Err(InstanceError::Authentication);
            }
        }
        // Recheck before setup; the storage transaction also fences concurrent Owner creation.
        if storage_has_owner(self.storage.as_ref()).await? {
            *self.state()? = InstanceState::Initialized;
            return Err(InstanceError::AlreadyInitialized);
        }
        let name = validated_name(&request.name)?;
        let email = normalize_email(&request.email)?;
        let password = request.password.clone();
        let password_hash = tokio::task::spawn_blocking(move || hash_password(&password)).await??;
        let owner = NewUser {
            name,
            email,
            password_hash,
            global_role: GlobalRole::Owner,
            mail_access: MailAccess::All,
            personal_filter: None,
        };
        match self.storage.create_first_owner(&owner).await {
            Ok(_) => {}
            Err(StorageError::InstanceAlreadyInitialized) => {
                *self.state()? = InstanceState::Initialized;
                return Err(InstanceError::AlreadyInitialized);
            }
            Err(error) => return Err(error.into()),
        }
        // Once the database has committed, every write failure must remove the usable secret.
        self.require_recovery()?;
        let metadata = InstanceMetadata {
            version: 1,
            instance_id: Uuid::new_v4(),
        };
        write_metadata(&self.directory, &metadata)?;
        *self.state()? = InstanceState::Initialized;
        Ok(InstanceStatus { initialized: true })
    }

    /// Move installation state to recovery, refusing to reopen first-run setup.
    fn require_recovery(&self) -> Result<(), InstanceError> {
        let mut state = self.state()?;
        *state = InstanceState::RecoveryRequired;
        Ok(())
    }

    /**
     * Validate credentials and create a remembered session or a 24-hour process session.
     */
    pub async fn login(&self, request: LoginRequest) -> Result<LoginResponse, InstanceError> {
        self.require_initialized()?;
        if request.password.len() > 1024 {
            return Err(InstanceError::Authentication);
        }
        let email = normalize_email(&request.email).map_err(|_| InstanceError::Authentication)?;
        let user = self
            .storage
            .get_user_by_email(&email)
            .await?
            .ok_or(InstanceError::Authentication)?;
        let password = request.password;
        let password_hash = user.password_hash.clone();
        if !tokio::task::spawn_blocking(move || verify_password(&password, &password_hash)).await? {
            return Err(InstanceError::Authentication);
        }
        let verified_password_hash = user.password_hash.clone();
        let session_user = session_user(user);
        let token = random_secret("SESSION")?;
        if request.remember_me {
            self.storage
                .create_persistent_session(
                    &token_fingerprint(&token),
                    session_user.id,
                    &verified_password_hash,
                )
                .await?;
        } else {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| InstanceError::RecoveryRequired)?;
            sessions.retain(|_, session| session.expires > Instant::now());
            if sessions.len() >= 128 {
                return Err(InstanceError::InvalidInput(
                    "too many active sessions".into(),
                ));
            }
            sessions.insert(
                token.clone(),
                Session {
                    user: session_user.clone(),
                    verified_password_hash,
                    expires: Instant::now() + Duration::from_secs(86400),
                },
            );
        }
        Ok(LoginResponse {
            token,
            user: session_user,
        })
    }

    /// Reject normal authentication before setup or while recovery is required.
    fn require_initialized(&self) -> Result<(), InstanceError> {
        if self.status()?.initialized {
            Ok(())
        } else {
            Err(InstanceError::SetupRequired)
        }
    }

    /// Authenticate a session token consistently for HTTP and native operations.
    pub async fn session(&self, token: &str) -> Result<SessionUser, InstanceError> {
        self.require_initialized()?;
        let process_session = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| InstanceError::RecoveryRequired)?;
            sessions.retain(|_, session| session.expires > Instant::now());
            sessions
                .get(token)
                .map(|session| (session.user.id, session.verified_password_hash.clone()))
        };
        let (identifier, verified_password_hash) = match process_session {
            Some(session) => session,
            None => self
                .storage
                .get_persistent_session(&token_fingerprint(token))
                .await?
                .ok_or(InstanceError::Authentication)?,
        };
        let user = self
            .storage
            .get_user(identifier)
            .await?
            .ok_or(InstanceError::Authentication)?;
        if user.password_hash != verified_password_hash {
            return Err(InstanceError::Authentication);
        }
        Ok(session_user(user))
    }

    /**
     * Identify remembered tokens so transports can renew their persistent cookies.
     */
    pub async fn is_persistent_session(&self, token: &str) -> Result<bool, InstanceError> {
        Ok(self
            .storage
            .get_persistent_session(&token_fingerprint(token))
            .await?
            .is_some())
    }

    /**
     * Revoke process-local and durable sessions after credentials or account status change.
     */
    pub(crate) async fn revoke_user_sessions(
        &self,
        identifier: UserIdentifier,
    ) -> Result<(), InstanceError> {
        self.sessions
            .lock()
            .map_err(|_| InstanceError::RecoveryRequired)?
            .retain(|_, session| session.user.id != identifier);
        self.storage
            .delete_user_persistent_sessions(identifier)
            .await?;
        Ok(())
    }

    /**
     * Revoke a session durably; already-revoked tokens remain harmless.
     */
    pub async fn logout(&self, token: &str) -> Result<(), InstanceError> {
        let process_session = self
            .sessions
            .lock()
            .map_err(|_| InstanceError::RecoveryRequired)?
            .remove(token);
        if process_session.is_none() {
            self.storage
                .delete_persistent_session(&token_fingerprint(token))
                .await?;
        }
        Ok(())
    }
}

/**
 * Hash a random bearer token before using it as a durable storage key.
 */
fn token_fingerprint(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Validate password bounds and generate an independently salted Argon2id hash.
pub(crate) fn hash_password(password: &str) -> Result<String, InstanceError> {
    if password.len() < 12 || password.len() > 1024 {
        return Err(InstanceError::InvalidInput(
            "password must contain 12 to 1024 bytes".into(),
        ));
    }
    Ok(Argon2::default()
        .hash_password(password.as_bytes())
        .map_err(|error| InstanceError::Cryptography(error.to_string()))?
        .to_string())
}

/// Check a password against a stored Argon2 PHC hash; malformed hashes never verify.
///
/// The parameters and salt come from the hash itself, so hashes created with older cost
/// parameters keep verifying.
pub(crate) fn verify_password(password: &str, password_hash: &str) -> bool {
    PasswordHash::new(password_hash).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}

/// Validate a bounded printable display name.
pub(crate) fn validated_name(value: &str) -> Result<String, InstanceError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 100 || value.chars().any(char::is_control) {
        return Err(InstanceError::InvalidInput(
            "name must contain 1 to 100 printable characters".into(),
        ));
    }
    Ok(value.to_owned())
}

/// Normalize a bounded email identity and reject controls, spaces, and ambiguous separators.
pub(crate) fn normalize_email(value: &str) -> Result<String, InstanceError> {
    let value = value.trim().to_ascii_lowercase();
    let valid = value.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty() && !domain.is_empty() && !domain.contains('@')
    });
    if !valid
        || value.len() > 254
        || value
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(InstanceError::InvalidInput(
            "enter a valid email address".into(),
        ));
    }
    Ok(value)
}

/// Convert a persisted user into the minimal session identity.
fn session_user(user: User) -> SessionUser {
    SessionUser {
        id: user.identifier,
        name: user.name,
        email: user.email,
        role: user.global_role,
    }
}

/// Obtain cryptographically secure operating-system random bytes, failing explicitly on error.
pub(crate) fn random_bytes<const LENGTH: usize>() -> Result<[u8; LENGTH], InstanceError> {
    let mut bytes = [0; LENGTH];
    getrandom::fill(&mut bytes).map_err(|error| InstanceError::Cryptography(error.to_string()))?;
    Ok(bytes)
}

/// Encode 256 random bits in copyable, grouped uppercase hexadecimal.
fn random_secret(prefix: &str) -> Result<String, InstanceError> {
    let bytes = random_bytes::<32>()?;
    let mut secret = String::from(prefix);
    for group in bytes.chunks(4) {
        secret.push('-');
        for byte in group {
            use std::fmt::Write;
            write!(&mut secret, "{byte:02X}").expect("string formatting cannot fail");
        }
    }
    Ok(secret)
}

/// Determine bootstrap availability from authoritative user roles.
async fn storage_has_owner(storage: &dyn Storage) -> Result<bool, StorageError> {
    Ok(storage
        .list_users()
        .await?
        .iter()
        .any(|user| user.global_role == GlobalRole::Owner))
}

/// Flush metadata to a same-directory temporary file, rename atomically, and sync its directory.
fn write_metadata(
    directory: &std::path::Path,
    metadata: &InstanceMetadata,
) -> Result<(), InstanceError> {
    let temporary_path = directory.join(format!("instance.json.{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary_path)?;
        let contents = serde_json::to_vec_pretty(metadata).map_err(std::io::Error::other)?;
        file.write_all(&contents)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary_path, directory.join("instance.json"))?;
        #[cfg(unix)]
        fs::File::open(directory)?.sync_all()?;
        Ok::<_, std::io::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary_path);
    }
    result.map_err(InstanceError::WriteMetadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandpost_storage_sqlite::SqliteStorage;

    /// Own an isolated on-disk installation and remove it after its storage handles close.
    struct Directory(PathBuf);

    impl Directory {
        /// Create a unique installation directory without touching repository data.
        fn new() -> Self {
            let directory = std::env::temp_dir()
                .join(format!("sandpost-bootstrap-lifecycle-{}", Uuid::new_v4()));
            fs::create_dir_all(&directory).unwrap();
            Self(directory)
        }

        /// Reopen the same durable database as a new runtime startup would.
        async fn storage(&self) -> Arc<dyn Storage> {
            Arc::new(
                SqliteStorage::open(self.0.join("sandpost.sqlite3"))
                    .await
                    .unwrap(),
            )
        }

        /// Check every persisted file, including SQLite journals, for leaked bootstrap secrets.
        fn assert_secrets_absent(&self, secrets: &[String]) {
            for entry in fs::read_dir(&self.0).unwrap() {
                let path = entry.unwrap().path();
                assert!(path.is_file(), "unexpected bootstrap artifact: {path:?}");
                let contents = fs::read(&path).unwrap();
                for secret in secrets {
                    assert!(
                        !contents
                            .windows(secret.len())
                            .any(|window| window == secret.as_bytes()),
                        "bootstrap secret persisted in {path:?}"
                    );
                }
            }
        }
    }

    impl Drop for Directory {
        /// Reclaim only this test's temporary installation.
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Supply valid owner details so secret rejection cannot be caused by invalid setup input.
    fn setup_request(secret: &str) -> SetupRequest {
        SetupRequest {
            bootstrap_secret: secret.into(),
            name: "Test Owner".into(),
            email: "owner@example.test".into(),
            password: "correct horse battery staple".into(),
        }
    }

    /**
     * Remembered tokens survive database reopening, remain hash-only, and logout is durable.
     */
    #[tokio::test]
    async fn remembered_sessions_survive_restart_until_logout() {
        let directory = Directory::new();
        let storage = directory.storage().await;
        let instance = Instance::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        let secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
        instance.setup(setup_request(&secret)).await.unwrap();
        let remembered = instance
            .login(LoginRequest {
                email: "owner@example.test".into(),
                password: "correct horse battery staple".into(),
                remember_me: true,
            })
            .await
            .unwrap();
        let temporary = instance
            .login(LoginRequest {
                email: "owner@example.test".into(),
                password: "correct horse battery staple".into(),
                remember_me: false,
            })
            .await
            .unwrap();
        directory.assert_secrets_absent(std::slice::from_ref(&remembered.token));
        assert!(instance.session(&temporary.token).await.is_ok());
        drop(instance);
        drop(storage);
        let instance = Instance::open(directory.storage().await, directory.0.clone())
            .await
            .unwrap();
        assert_eq!(
            instance.session(&remembered.token).await.unwrap().id,
            remembered.user.id
        );
        assert!(matches!(
            instance.session(&temporary.token).await,
            Err(InstanceError::Authentication)
        ));
        instance.logout(&remembered.token).await.unwrap();
        drop(instance);
        let restarted = Instance::open(directory.storage().await, directory.0.clone())
            .await
            .unwrap();
        assert!(matches!(
            restarted.session(&remembered.token).await,
            Err(InstanceError::Authentication)
        ));
    }

    /**
     * Password changes and explicit account revocation reject remembered tokens after restart.
     */
    #[tokio::test]
    async fn remembered_sessions_remain_bound_to_credentials_and_account_revocation() {
        let directory = Directory::new();
        let storage = directory.storage().await;
        let instance = Instance::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        let secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
        instance.setup(setup_request(&secret)).await.unwrap();
        let login = instance
            .login(LoginRequest {
                email: "owner@example.test".into(),
                password: "correct horse battery staple".into(),
                remember_me: true,
            })
            .await
            .unwrap();
        let user = storage.get_user(login.user.id).await.unwrap().unwrap();
        storage
            .update_user(
                user.identifier,
                &sandpost_storage::UpdateUser {
                    name: user.name,
                    email: user.email,
                    password_hash: hash_password("a different secure password").unwrap(),
                    global_role: user.global_role,
                    mail_access: user.mail_access,
                    personal_filter: user.personal_filter,
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            instance.session(&login.token).await,
            Err(InstanceError::Authentication)
        ));
        let login = instance
            .login(LoginRequest {
                email: "owner@example.test".into(),
                password: "a different secure password".into(),
                remember_me: true,
            })
            .await
            .unwrap();
        instance.revoke_user_sessions(login.user.id).await.unwrap();
        drop(instance);
        let restarted = Instance::open(storage, directory.0.clone()).await.unwrap();
        assert!(matches!(
            restarted.session(&login.token).await,
            Err(InstanceError::Authentication)
        ));
    }

    /// Each unconfigured startup rotates its memory-only key and refuses all previous keys.
    #[tokio::test]
    async fn uninitialized_boots_rotate_secrets_without_persisting_them() {
        let directory = Directory::new();
        let mut previous_secrets = Vec::new();
        let mut previous_authorizations = Vec::<String>::new();
        for _ in 0..3 {
            let storage = directory.storage().await;
            let instance = Instance::open(storage.clone(), directory.0.clone())
                .await
                .unwrap();
            let secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
            assert!(!instance.status().unwrap().initialized);
            assert!(!previous_secrets.contains(&secret));
            assert_eq!(
                instance.bootstrap_secret_for_startup().unwrap().as_deref(),
                Some(secret.as_str()),
                "the key must remain stable within one startup"
            );
            for previous_secret in &previous_secrets {
                assert!(matches!(
                    instance.authorize_setup(previous_secret),
                    Err(InstanceError::Authentication)
                ));
                assert!(matches!(
                    instance.setup(setup_request(previous_secret)).await,
                    Err(InstanceError::Authentication)
                ));
            }
            for authorization in &previous_authorizations {
                assert!(matches!(
                    instance.setup(setup_request(authorization)).await,
                    Err(InstanceError::Authentication)
                ));
            }
            assert!(matches!(
                instance.authorize_setup("wrong-key"),
                Err(InstanceError::Authentication)
            ));
            previous_authorizations.push(instance.authorize_setup(&secret).unwrap());
            assert_eq!(storage.count_users().await.unwrap(), 0);
            previous_secrets.push(secret);
            assert!(!directory.0.join("instance.json").exists());
            directory.assert_secrets_absent(&previous_secrets);
            directory.assert_secrets_absent(&previous_authorizations);
            drop(instance);
            drop(storage);
            directory.assert_secrets_absent(&previous_secrets);
        }
    }

    /// Successful setup removes the key, persists only identity, and stays locked after restart.
    #[tokio::test]
    async fn completed_setup_never_reopens_bootstrap_after_restart() {
        let directory = Directory::new();
        let storage = directory.storage().await;
        let instance = Instance::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        let secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
        assert!(
            instance
                .setup(setup_request(&secret))
                .await
                .unwrap()
                .initialized
        );
        assert_eq!(storage.count_users().await.unwrap(), 1);
        let owner = storage
            .get_user_by_email("owner@example.test")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(owner.global_role, GlobalRole::Owner);
        assert!(instance.bootstrap_secret_for_startup().unwrap().is_none());
        for submitted_secret in [&secret, "wrong-secret"] {
            assert!(matches!(
                instance.setup(setup_request(submitted_secret)).await,
                Err(InstanceError::AlreadyInitialized)
            ));
        }
        let metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.0.join("instance.json")).unwrap()).unwrap();
        assert_eq!(metadata.as_object().unwrap().len(), 2);
        assert_eq!(metadata["version"], 1);
        assert!(
            !Uuid::parse_str(metadata["instanceId"].as_str().unwrap())
                .unwrap()
                .is_nil()
        );
        directory.assert_secrets_absent(std::slice::from_ref(&secret));
        drop(instance);
        drop(storage);

        let storage = directory.storage().await;
        let restarted = Instance::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        assert!(restarted.status().unwrap().initialized);
        assert!(restarted.bootstrap_secret_for_startup().unwrap().is_none());
        for submitted_secret in [&secret, "wrong-secret"] {
            assert!(matches!(
                restarted.setup(setup_request(submitted_secret)).await,
                Err(InstanceError::AlreadyInitialized)
            ));
        }
        assert_eq!(storage.count_users().await.unwrap(), 1);
        directory.assert_secrets_absent(std::slice::from_ref(&secret));
    }

    /// Verification creates no owner; its opaque credential completes setup and dies on transition.
    #[tokio::test]
    async fn setup_authorization_completes_setup_without_persisting_credentials() {
        let directory = Directory::new();
        let storage = directory.storage().await;
        let instance = Instance::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        let secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
        let authorization = instance.authorize_setup(&secret).unwrap();
        let unused_authorization = instance.authorize_setup(&secret).unwrap();
        assert_ne!(authorization, secret);
        assert_ne!(authorization, unused_authorization);
        assert_eq!(storage.count_users().await.unwrap(), 0);
        assert!(!directory.0.join("instance.json").exists());
        assert_eq!(
            serde_json::to_value(instance.status().unwrap()).unwrap(),
            serde_json::json!({ "initialized": false })
        );
        assert!(matches!(
            instance.authorize_setup(&authorization),
            Err(InstanceError::Authentication)
        ));
        assert!(
            instance
                .setup(setup_request(&authorization))
                .await
                .unwrap()
                .initialized
        );
        assert_eq!(storage.count_users().await.unwrap(), 1);
        assert!(matches!(
            &*instance.state().unwrap(),
            InstanceState::Initialized
        ));
        let credentials = [secret, authorization, unused_authorization];
        for credential in &credentials {
            assert!(matches!(
                instance.authorize_setup(credential),
                Err(InstanceError::AlreadyInitialized)
            ));
            assert!(matches!(
                instance.setup(setup_request(credential)).await,
                Err(InstanceError::AlreadyInitialized)
            ));
        }
        directory.assert_secrets_absent(&credentials);
        drop(instance);
        drop(storage);

        let restarted = Instance::open(directory.storage().await, directory.0.clone())
            .await
            .unwrap();
        for credential in &credentials {
            assert!(matches!(
                restarted.authorize_setup(credential),
                Err(InstanceError::AlreadyInitialized)
            ));
            assert!(matches!(
                restarted.setup(setup_request(credential)).await,
                Err(InstanceError::AlreadyInitialized)
            ));
        }
        directory.assert_secrets_absent(&credentials);
    }

    /// Invalid, expired, and evicted authorizations cannot create an owner; the boot key survives.
    #[tokio::test]
    async fn setup_authorizations_are_bounded_and_expire() {
        let directory = Directory::new();
        let storage = directory.storage().await;
        let instance = Instance::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        let secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
        let expired_authorization = instance.authorize_setup(&secret).unwrap();
        {
            let mut state = instance.state().unwrap();
            let InstanceState::Bootstrap { authorizations, .. } = &mut *state else {
                panic!("verification must leave bootstrap active");
            };
            let remaining = authorizations[0].1.duration_since(Instant::now());
            assert!(remaining <= Duration::from_secs(600));
            assert!(remaining > Duration::from_secs(590));
            authorizations[0].1 = Instant::now() - Duration::from_secs(1);
        }
        for credential in [&expired_authorization, "invalid-authorization", ""] {
            assert!(matches!(
                instance.setup(setup_request(credential)).await,
                Err(InstanceError::Authentication)
            ));
        }
        let oldest_authorization = instance.authorize_setup(&secret).unwrap();
        let mut newest_authorization = String::new();
        for _ in 0..64 {
            newest_authorization = instance.authorize_setup(&secret).unwrap();
        }
        {
            let state = instance.state().unwrap();
            let InstanceState::Bootstrap { authorizations, .. } = &*state else {
                panic!("verification must leave bootstrap active");
            };
            assert_eq!(authorizations.len(), 64);
            assert_eq!(authorizations.last().unwrap().0, newest_authorization);
        }
        assert!(matches!(
            instance.setup(setup_request(&oldest_authorization)).await,
            Err(InstanceError::Authentication)
        ));
        assert_eq!(storage.count_users().await.unwrap(), 0);
        assert_eq!(
            instance.bootstrap_secret_for_startup().unwrap().unwrap(),
            secret
        );
        assert!(
            instance
                .setup(setup_request(&newest_authorization))
                .await
                .unwrap()
                .initialized
        );
    }

    /// Racing key/token setup and verification retain one owner and no usable authorizations.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn racing_setup_and_verification_have_one_winner() {
        let directory = Directory::new();
        let storage = directory.storage().await;
        let instance = Instance::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        let secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
        let authorization = instance.authorize_setup(&secret).unwrap();
        let competing_instance = instance.clone();
        let verification_instance = instance.clone();
        let verification_key = secret.clone();
        let (first, second, verification) = tokio::join!(
            tokio::spawn(async move {
                competing_instance
                    .setup(setup_request(&authorization))
                    .await
            }),
            instance.setup(setup_request(&secret)),
            tokio::spawn(async move { verification_instance.authorize_setup(&verification_key) }),
        );
        let first = first.unwrap();
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        let losing_result = if first.is_err() { first } else { second };
        assert!(matches!(
            losing_result,
            Err(InstanceError::AlreadyInitialized)
        ));
        match verification.unwrap() {
            Ok(authorization) => assert!(matches!(
                instance.setup(setup_request(&authorization)).await,
                Err(InstanceError::AlreadyInitialized)
            )),
            Err(InstanceError::AlreadyInitialized) => {}
            Err(error) => panic!("unexpected verification failure: {error}"),
        }
        assert_eq!(storage.count_users().await.unwrap(), 1);
        assert!(matches!(
            instance.authorize_setup(&secret),
            Err(InstanceError::AlreadyInitialized)
        ));
    }

    /// Owner presence controls setup even with stale or malformed installation metadata.
    #[tokio::test]
    async fn owner_presence_controls_setup_regardless_of_metadata() {
        for role in [None, Some(GlobalRole::Member), Some(GlobalRole::Owner)] {
            for metadata in [
                None,
                Some(r#"{"version":1,"instanceId":"00000000-0000-0000-0000-000000000001"}"#),
                Some("invalid metadata"),
            ] {
                let directory = Directory::new();
                let storage = directory.storage().await;
                let existing_user = if let Some(role) = role {
                    Some(
                        storage
                            .create_user(&NewUser {
                                name: "Existing user".into(),
                                email: "existing@example.test".into(),
                                password_hash: "unchanged existing password hash".into(),
                                global_role: role,
                                mail_access: MailAccess::All,
                                personal_filter: None,
                            })
                            .await
                            .unwrap(),
                    )
                } else {
                    None
                };
                if let Some(metadata) = metadata {
                    fs::write(directory.0.join("instance.json"), metadata).unwrap();
                }
                let instance = Instance::open(storage.clone(), directory.0.clone())
                    .await
                    .unwrap();
                assert_eq!(
                    instance.status().unwrap().initialized,
                    role == Some(GlobalRole::Owner)
                );
                if role == Some(GlobalRole::Owner) {
                    assert!(instance.bootstrap_secret_for_startup().unwrap().is_none());
                } else {
                    let secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
                    assert!(
                        instance
                            .setup(setup_request(&secret))
                            .await
                            .unwrap()
                            .initialized
                    );
                    assert!(storage_has_owner(storage.as_ref()).await.unwrap());
                }
                if let Some(existing_user) = existing_user {
                    let retained = storage
                        .get_user(existing_user.identifier)
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(retained.password_hash, existing_user.password_hash);
                    assert_eq!(retained.global_role, existing_user.global_role);
                }
            }
        }
    }

    /// Independent bootstrap runtimes cannot commit two first Owners to the same database.
    #[tokio::test]
    async fn independent_setups_commit_only_one_first_owner() {
        let directory = Directory::new();
        let storage = directory.storage().await;
        let first = Instance::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        let second = Instance::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        let first_secret = first.bootstrap_secret_for_startup().unwrap().unwrap();
        let second_secret = second.bootstrap_secret_for_startup().unwrap().unwrap();
        let (first_result, second_result) = tokio::join!(
            first.setup(setup_request(&first_secret)),
            second.setup(setup_request(&second_secret)),
        );
        assert_eq!(
            usize::from(first_result.is_ok()) + usize::from(second_result.is_ok()),
            1
        );
        let losing_result = if first_result.is_err() {
            first_result
        } else {
            second_result
        };
        assert!(matches!(
            losing_result,
            Err(InstanceError::AlreadyInitialized)
        ));
        assert_eq!(storage.count_users().await.unwrap(), 1);
        assert!(first.status().unwrap().initialized);
        assert!(second.status().unwrap().initialized);
        assert!(first.bootstrap_secret_for_startup().unwrap().is_none());
        assert!(second.bootstrap_secret_for_startup().unwrap().is_none());
    }

    /// A committed Owner closes setup and permits restart even without installation metadata.
    #[tokio::test]
    async fn first_owner_without_metadata_closes_setup_and_initializes_restart() {
        let directory = Directory::new();
        let storage = directory.storage().await;
        let instance = Instance::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        let secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
        let authorization = instance.authorize_setup(&secret).unwrap();
        storage
            .create_first_owner(&NewUser {
                name: "Test Owner".into(),
                email: "owner@example.test".into(),
                password_hash: "unused in this bootstrap-only test".into(),
                global_role: GlobalRole::Owner,
                mail_access: MailAccess::All,
                personal_filter: None,
            })
            .await
            .unwrap();
        assert!(matches!(
            instance.setup(setup_request(&authorization)).await,
            Err(InstanceError::AlreadyInitialized)
        ));
        assert!(instance.bootstrap_secret_for_startup().unwrap().is_none());
        assert!(matches!(
            instance.authorize_setup(&secret),
            Err(InstanceError::AlreadyInitialized)
        ));
        assert!(matches!(
            instance.setup(setup_request(&authorization)).await,
            Err(InstanceError::AlreadyInitialized)
        ));
        assert!(matches!(
            &*instance.state().unwrap(),
            InstanceState::Initialized
        ));
        drop(instance);
        drop(storage);

        let restarted = Instance::open(directory.storage().await, directory.0.clone())
            .await
            .unwrap();
        assert!(restarted.status().unwrap().initialized);
        assert!(restarted.bootstrap_secret_for_startup().unwrap().is_none());
        assert!(!directory.0.join("instance.json").exists());
        directory.assert_secrets_absent(std::slice::from_ref(&secret));
        directory.assert_secrets_absent(std::slice::from_ref(&authorization));
    }
}
