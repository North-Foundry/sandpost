//! SMTP access credentials and authentication rule conformance checks.
use sandpost_core::{SmtpAccessIdentifier, SmtpAuthenticationMechanism};
use sandpost_storage::{NewSmtpAccess, Storage, StorageError};

use crate::ConformanceFailure;
use crate::support::{failure, unique_token, unwrap_storage, verify, verify_equal};

/// Build a new access request with a process-unique username.
fn new_smtp_access_request(password_hash: &str) -> NewSmtpAccess {
    NewSmtpAccess {
        name: "conformance access".to_owned(),
        username: format!("conformance-{}-{}", std::process::id(), unique_token()),
        password_hash: password_hash.to_owned(),
        created_at: 1_700_000_000,
        requires_encryption: false,
        allowed_mechanisms: SmtpAuthenticationMechanism::ALL.to_vec(),
    }
}

/// Verify an access stores its TLS requirement and AUTH mechanisms, and refuses an empty set.
pub async fn smtp_access_authentication_rules(
    storage: &dyn Storage,
) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "smtp_access_authentication_rules";
    let created = unwrap_storage(
        CHECK,
        "create_smtp_access with rules",
        storage
            .create_smtp_access(&NewSmtpAccess {
                requires_encryption: true,
                allowed_mechanisms: vec![SmtpAuthenticationMechanism::Login],
                ..new_smtp_access_request("rules-hash")
            })
            .await,
    )?;
    verify(
        CHECK,
        created.requires_encryption,
        "the created access must require encryption",
    )?;
    verify_equal(
        CHECK,
        "created mechanisms",
        created.allowed_mechanisms.clone(),
        vec![SmtpAuthenticationMechanism::Login],
    )?;
    let changed = unwrap_storage(
        CHECK,
        "set_smtp_access_authentication_rules",
        storage
            .set_smtp_access_authentication_rules(
                created.identifier,
                false,
                &[
                    SmtpAuthenticationMechanism::Login,
                    SmtpAuthenticationMechanism::Plain,
                ],
            )
            .await,
    )?
    .ok_or_else(|| {
        failure(
            CHECK,
            "changing the rules of an existing access returned none",
        )
    })?;
    verify(
        CHECK,
        !changed.requires_encryption,
        "the TLS requirement must be cleared",
    )?;
    verify_equal(
        CHECK,
        "mechanisms in canonical order",
        changed.allowed_mechanisms.clone(),
        SmtpAuthenticationMechanism::ALL.to_vec(),
    )?;
    let credential = unwrap_storage(
        CHECK,
        "get_smtp_access_credential after rule change",
        storage.get_smtp_access_credential(created.identifier).await,
    )?
    .ok_or_else(|| failure(CHECK, "the credential disappeared"))?;
    verify_equal(
        CHECK,
        "credential carries the rules",
        credential.access,
        changed,
    )?;
    let empty = storage
        .set_smtp_access_authentication_rules(created.identifier, false, &[])
        .await;
    verify(
        CHECK,
        matches!(empty, Err(StorageError::ConstraintViolation(_))),
        format!("an empty mechanism set must be a constraint violation, got {empty:?}"),
    )?;
    let empty_creation = storage
        .create_smtp_access(&NewSmtpAccess {
            allowed_mechanisms: Vec::new(),
            ..new_smtp_access_request("empty-hash")
        })
        .await;
    verify(
        CHECK,
        matches!(empty_creation, Err(StorageError::ConstraintViolation(_))),
        format!(
            "creating without mechanisms must be a constraint violation, got {empty_creation:?}"
        ),
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "set_smtp_access_authentication_rules unknown",
            storage
                .set_smtp_access_authentication_rules(
                    SmtpAccessIdentifier::new(),
                    true,
                    &SmtpAuthenticationMechanism::ALL,
                )
                .await,
        )?
        .is_none(),
        "changing the rules of an unknown access must return none",
    )?;
    unwrap_storage(
        CHECK,
        "delete_smtp_access",
        storage.delete_smtp_access(created.identifier).await,
    )?;
    Ok(())
}

/// Verify SMTP access creation, credential reads, password replacement, status, use tracking,
/// and deletion.
pub async fn smtp_access_credentials(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "smtp_access_credentials";
    let request = new_smtp_access_request("first-hash");
    let created = unwrap_storage(
        CHECK,
        "create_smtp_access",
        storage.create_smtp_access(&request).await,
    )?;
    verify(CHECK, created.enabled, "a new SMTP access must be enabled")?;
    verify_equal(
        CHECK,
        "created username",
        created.username.as_str(),
        request.username.as_str(),
    )?;
    verify_equal(CHECK, "created_at", created.created_at, request.created_at)?;
    verify_equal(CHECK, "last_used_at", created.last_used_at, None)?;
    verify_equal(
        CHECK,
        "get_smtp_access",
        unwrap_storage(
            CHECK,
            "get_smtp_access",
            storage.get_smtp_access(created.identifier).await,
        )?,
        Some(created.clone()),
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "list_smtp_accesses",
            storage.list_smtp_accesses().await,
        )?
        .contains(&created),
        "list_smtp_accesses omitted a created access",
    )?;
    let credential = unwrap_storage(
        CHECK,
        "get_smtp_access_credential_by_username",
        storage
            .get_smtp_access_credential_by_username(&request.username)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "credential lookup by username returned none"))?;
    verify_equal(
        CHECK,
        "credential access",
        credential.access.clone(),
        created.clone(),
    )?;
    verify_equal(
        CHECK,
        "stored hash",
        credential.password_hash.as_str(),
        "first-hash",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "credential lookup for unknown username",
            storage
                .get_smtp_access_credential_by_username("missing-conformance-user")
                .await,
        )?
        .is_none(),
        "an unknown username must not resolve to a credential",
    )?;

    let duplicate = storage
        .create_smtp_access(&NewSmtpAccess {
            username: request.username.clone(),
            ..new_smtp_access_request("other-hash")
        })
        .await;
    verify(
        CHECK,
        matches!(duplicate, Err(StorageError::DuplicateSmtpUsername)),
        format!("a duplicate username must fail with DuplicateSmtpUsername, got {duplicate:?}"),
    )?;
    let replaced = unwrap_storage(
        CHECK,
        "replace_smtp_access_password",
        storage
            .replace_smtp_access_password(created.identifier, "second-hash")
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "replacing an existing password returned none"))?;
    verify_equal(
        CHECK,
        "replaced access identity",
        replaced.identifier,
        created.identifier,
    )?;
    verify_equal(
        CHECK,
        "hash after replacement",
        unwrap_storage(
            CHECK,
            "get_smtp_access_credential",
            storage.get_smtp_access_credential(created.identifier).await,
        )?
        .map(|credential| credential.password_hash),
        Some("second-hash".to_owned()),
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "replace password of unknown access",
            storage
                .replace_smtp_access_password(SmtpAccessIdentifier::new(), "hash")
                .await,
        )?
        .is_none(),
        "replacing the password of an unknown access must return none",
    )?;

    let disabled = unwrap_storage(
        CHECK,
        "set_smtp_access_enabled false",
        storage
            .set_smtp_access_enabled(created.identifier, false)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "disabling an existing access returned none"))?;
    verify(
        CHECK,
        !disabled.enabled,
        "a disabled access must report enabled = false",
    )?;
    let enabled = unwrap_storage(
        CHECK,
        "set_smtp_access_enabled true",
        storage
            .set_smtp_access_enabled(created.identifier, true)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "enabling an existing access returned none"))?;
    verify(
        CHECK,
        enabled.enabled,
        "a re-enabled access must report enabled = true",
    )?;

    unwrap_storage(
        CHECK,
        "record_smtp_access_use",
        storage
            .record_smtp_access_use(created.identifier, 1_700_000_100)
            .await,
    )?;
    let used = unwrap_storage(
        CHECK,
        "get_smtp_access after use",
        storage.get_smtp_access(created.identifier).await,
    )?
    .ok_or_else(|| failure(CHECK, "used access disappeared"))?;
    verify_equal(
        CHECK,
        "last_used_at",
        used.last_used_at,
        Some(1_700_000_100),
    )?;
    unwrap_storage(
        CHECK,
        "record_smtp_access_use later",
        storage
            .record_smtp_access_use(created.identifier, 1_700_000_200)
            .await,
    )?;
    verify_equal(
        CHECK,
        "last_used_at after a later use",
        unwrap_storage(
            CHECK,
            "get_smtp_access after later use",
            storage.get_smtp_access(created.identifier).await,
        )?
        .and_then(|access| access.last_used_at),
        Some(1_700_000_200),
    )?;

    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "delete_smtp_access",
            storage.delete_smtp_access(created.identifier).await,
        )?,
        "delete_smtp_access must report true for an existing access",
    )?;
    verify(
        CHECK,
        !unwrap_storage(
            CHECK,
            "delete_smtp_access twice",
            storage.delete_smtp_access(created.identifier).await,
        )?,
        "delete_smtp_access must report false for a missing access",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "credential after delete",
            storage
                .get_smtp_access_credential_by_username(&request.username)
                .await,
        )?
        .is_none(),
        "a deleted access must not authenticate",
    )
}
