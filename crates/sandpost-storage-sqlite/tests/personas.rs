//! SQLite-specific Persona guarantees: database-enforced address uniqueness under concurrency,
//! raw constraint enforcement, and atomic scheduling with message insertion.
#[allow(dead_code)]
mod common;

use sandpost_core::{GlobalRole, MailAccess, Mailbox, PersonaIdentifier};
use sandpost_query::Expression;
use sandpost_storage::{
    MessageStorage, NewPersona, NewUser, PersonaStorage, StorageError, UserStorage,
};
use sandpost_storage_sqlite::SqliteStorage;
use std::path::PathBuf;
use std::sync::Arc;

/// Own an isolated temporary directory holding one database file.
struct TestDirectory(PathBuf);

impl TestDirectory {
    /**
     * Create an isolated directory for a file database.
     */
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("sandpost-personas-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /**
     * Return the database path inside the directory.
     */
    fn database(&self) -> PathBuf {
        self.0.join("sandpost.sqlite3")
    }
}

impl Drop for TestDirectory {
    /**
     * Remove the database and its WAL sidecars.
     */
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/**
 * Create a member who owns test Personas.
 */
async fn member(storage: &SqliteStorage, email: &str) -> sandpost_core::User {
    storage
        .create_user(&NewUser {
            name: "Member".into(),
            email: email.into(),
            password_hash: "hash".into(),
            global_role: GlobalRole::Member,
            mail_access: MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap()
}

/**
 * Build a new Persona definition.
 */
fn persona(owner: sandpost_core::UserIdentifier, address: &str) -> NewPersona {
    NewPersona {
        identifier: PersonaIdentifier::new(),
        owner_identifier: owner,
        name: "Alice".into(),
        address: address.into(),
        icon: None,
        script: "open;".into(),
        enabled: true,
        created_at: 1,
    }
}

/**
 * Concurrent creations by different owners cannot both claim one address.
 */
#[tokio::test]
async fn concurrent_creation_cannot_bypass_address_uniqueness() {
    let directory = TestDirectory::new();
    let storage = Arc::new(SqliteStorage::open(&directory.database()).await.unwrap());
    let mut owners = Vec::new();
    for index in 0..8 {
        owners.push(member(&storage, &format!("owner{index}@example.test")).await);
    }
    let mut tasks = tokio::task::JoinSet::new();
    for owner in owners {
        let storage = storage.clone();
        tasks.spawn(async move {
            storage
                .create_persona(&persona(owner.identifier, "franco@ecommerce.it"))
                .await
        });
    }
    let mut created = 0;
    let mut duplicates = 0;
    while let Some(result) = tasks.join_next().await {
        match result.unwrap() {
            Ok(_) => created += 1,
            Err(StorageError::DuplicatePersonaAddress) => duplicates += 1,
            Err(error) => panic!("unexpected error {error}"),
        }
    }
    assert_eq!((created, duplicates), (1, 7));
    assert_eq!(storage.list_personas(None).await.unwrap().len(), 1);
    storage.close().await;
}

/**
 * The schema refuses duplicate and unnormalized addresses independent of the Rust code.
 */
#[tokio::test]
async fn schema_enforces_normalized_unique_addresses() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let storage = SqliteStorage::open(&path).await.unwrap();
    let owner = member(&storage, "owner@example.test").await;
    storage.close().await;
    let mut connection = common::connection(&path).await;
    let insert = |address: &'static str| {
        format!(
            "INSERT INTO personas(identifier,owner_identifier,name,address,script,enabled,created_at,updated_at) VALUES ('{}','{}','x','{address}','',1,1,1)",
            uuid::Uuid::new_v4(),
            owner.identifier
        )
    };
    sqlx::raw_sql(sqlx::AssertSqlSafe(insert("alice@example.test")))
        .execute(&mut connection)
        .await
        .unwrap();
    for rejected in [
        "alice@example.test",
        "Alice@Example.test",
        " bob@example.test",
        "",
    ] {
        let result = sqlx::raw_sql(sqlx::AssertSqlSafe(insert(rejected)))
            .execute(&mut connection)
            .await;
        assert!(
            result.is_err(),
            "{rejected:?} must be refused by the schema"
        );
    }
}

/**
 * Persist Persona icons through the storage API, preserve them across enable changes, and reset them.
 */
#[tokio::test]
async fn icon_persists_through_create_update_and_reset() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let storage = SqliteStorage::open(&path).await.unwrap();
    let owner = member(&storage, "owner@example.test").await;
    let mut definition = persona(owner.identifier, "alice@example.test");
    definition.icon = Some("venetian-mask".into());
    let created = storage.create_persona(&definition).await.unwrap();
    assert_eq!(created.icon.as_deref(), Some("venetian-mask"));
    assert_eq!(
        storage.get_persona(created.identifier).await.unwrap(),
        Some(created.clone())
    );

    for icon in [Some("bug"), None, Some("venetian-mask")] {
        let updated = storage
            .update_persona(
                created.identifier,
                &sandpost_storage::PersonaUpdate {
                    name: created.name.clone(),
                    address: created.address.clone(),
                    icon: icon.map(String::from),
                    script: created.script.clone(),
                    enabled: created.enabled,
                },
                2,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.icon.as_deref(), icon);
        assert_eq!(updated.revision, created.revision);
        assert_eq!(updated.updated_at, 2);
        assert_eq!(
            storage.get_persona(created.identifier).await.unwrap(),
            Some(updated)
        );
    }

    let disabled = storage
        .update_persona(
            created.identifier,
            &sandpost_storage::PersonaUpdate {
                name: created.name.clone(),
                address: created.address.clone(),
                icon: created.icon.clone(),
                script: created.script.clone(),
                enabled: false,
            },
            2,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(disabled.icon.as_deref(), Some("venetian-mask"));
    assert_eq!(disabled.revision, created.revision + 1);
    assert!(!disabled.enabled);

    let reset = storage
        .update_persona(
            created.identifier,
            &sandpost_storage::PersonaUpdate {
                name: disabled.name.clone(),
                address: disabled.address.clone(),
                icon: None,
                script: disabled.script.clone(),
                enabled: true,
            },
            3,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reset.icon, None);
    storage.close().await;

    let reopened = SqliteStorage::open(&path).await.unwrap();
    assert_eq!(
        reopened.get_persona(created.identifier).await.unwrap(),
        Some(reset)
    );
    reopened.close().await;
}

/**
 * Scheduling commits with the message: a failed insertion schedules nothing, a duplicate
 * message identifier schedules nothing twice, and scheduled executions survive reopening.
 */
#[tokio::test]
async fn scheduling_is_atomic_and_durable() {
    let directory = TestDirectory::new();
    let path = directory.database();
    let storage = SqliteStorage::open(&path).await.unwrap();
    let owner = member(&storage, "owner@example.test").await;
    let created = storage
        .create_persona(&persona(owner.identifier, "alice@example.test"))
        .await
        .unwrap();
    let mut message = common::message("Verify");
    message.facts.envelope_to = vec![Mailbox {
        address: "Alice@example.test".into(),
        domain: "example.test".into(),
    }];
    storage.insert_message(&message).await.unwrap();
    assert!(matches!(
        storage.insert_message(&message).await,
        Err(StorageError::DuplicateMessageIdentifier(_))
    ));
    storage.close().await;

    let reopened = SqliteStorage::open(&path).await.unwrap();
    let executions = reopened
        .list_persona_executions(
            created.identifier,
            owner.identifier,
            &Expression::True,
            None,
            10,
        )
        .await
        .unwrap();
    assert_eq!(
        executions.len(),
        1,
        "the execution is durable and scheduled once"
    );
    assert_eq!(
        executions[0].execution.message_identifier,
        message.identifier
    );
    reopened.close().await;
}
