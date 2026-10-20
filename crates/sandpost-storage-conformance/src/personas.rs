//! Persona definition, execution scheduling, lifecycle, retention, and history checks.
use sandpost_core::{
    GlobalRole, MessageIdentifier, Persona, PersonaActionDetail, PersonaActionStatus,
    PersonaExecution, PersonaExecutionAction, PersonaExecutionStatus, PersonaIdentifier,
    UserIdentifier,
};
use sandpost_query::{Expression, Field, Operator, Predicate, Value};
use sandpost_storage::{
    NewPersona, PERSONA_EXECUTION_RETENTION, PERSONA_PENDING_EXECUTION_LIMIT, PersonaUpdate,
    Storage, StorageError,
};

use crate::ConformanceFailure;
use crate::support::{
    create_user, failure, mailbox, simple_message, unique_email, unique_token, unwrap_storage,
    verify, verify_equal,
};

/**
 * Build a unique normalized Persona address.
 */
fn unique_address(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}@personas.test",
        std::process::id(),
        unique_token()
    )
}

/**
 * Create an enabled Persona for an owner with a unique address.
 */
async fn create_persona(
    storage: &dyn Storage,
    check: &'static str,
    owner: UserIdentifier,
    address: &str,
    enabled: bool,
) -> Result<Persona, ConformanceFailure> {
    unwrap_storage(
        check,
        "create_persona",
        storage
            .create_persona(&NewPersona {
                identifier: PersonaIdentifier::new(),
                owner_identifier: owner,
                name: "Conformance".into(),
                address: address.into(),
                script: "open;".into(),
                enabled,
                created_at: 100,
            })
            .await,
    )
}

/**
 * Store a message whose SMTP envelope lists the supplied recipients.
 */
async fn deliver(
    storage: &dyn Storage,
    check: &'static str,
    recipients: &[&str],
) -> Result<MessageIdentifier, ConformanceFailure> {
    let mut message = simple_message("persona delivery");
    message.facts.envelope_to = recipients.iter().map(|address| mailbox(address)).collect();
    message.facts.received_at = 500;
    unwrap_storage(
        check,
        "insert_message",
        storage.insert_message(&message).await,
    )?;
    Ok(message.identifier)
}

/**
 * Return every execution of a Persona visible through an always-true filter.
 */
async fn history(
    storage: &dyn Storage,
    check: &'static str,
    persona: PersonaIdentifier,
    viewer: UserIdentifier,
) -> Result<Vec<PersonaExecution>, ConformanceFailure> {
    Ok(unwrap_storage(
        check,
        "list_persona_executions",
        storage
            .list_persona_executions(persona, viewer, &Expression::True, None, 100)
            .await,
    )?
    .into_iter()
    .map(|entry| entry.execution)
    .collect())
}

/**
 * Verify Persona creation, reads, updates, global address uniqueness, deletion and owner cascade.
 *
 * Addresses are unique across every owner, including on update; an unknown owner is NotFound;
 * updates advance the revision; deleting the owner deletes the Persona.
 */
pub async fn persona_definitions(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "persona_definitions";
    let first_owner = create_user(
        storage,
        CHECK,
        GlobalRole::Member,
        &unique_email("persona-a"),
    )
    .await?;
    let second_owner = create_user(
        storage,
        CHECK,
        GlobalRole::Member,
        &unique_email("persona-b"),
    )
    .await?;
    let address = unique_address("alice");
    let persona = create_persona(storage, CHECK, first_owner.identifier, &address, true).await?;
    verify_equal(CHECK, "revision", persona.revision, 1)?;
    verify_equal(CHECK, "updated_at", persona.updated_at, 100)?;
    verify_equal(
        CHECK,
        "get_persona",
        unwrap_storage(
            CHECK,
            "get_persona",
            storage.get_persona(persona.identifier).await,
        )?,
        Some(persona.clone()),
    )?;

    let duplicate = storage
        .create_persona(&NewPersona {
            identifier: PersonaIdentifier::new(),
            owner_identifier: second_owner.identifier,
            name: "Other".into(),
            address: address.clone(),
            script: String::new(),
            enabled: true,
            created_at: 1,
        })
        .await;
    verify(
        CHECK,
        matches!(duplicate, Err(StorageError::DuplicatePersonaAddress)),
        format!("another owner reusing an address must fail, got {duplicate:?}"),
    )?;
    let unknown_owner = storage
        .create_persona(&NewPersona {
            identifier: PersonaIdentifier::new(),
            owner_identifier: UserIdentifier::new(),
            name: "Orphan".into(),
            address: unique_address("orphan"),
            script: String::new(),
            enabled: true,
            created_at: 1,
        })
        .await;
    verify(
        CHECK,
        matches!(unknown_owner, Err(StorageError::NotFound)),
        format!("an unknown owner must be NotFound, got {unknown_owner:?}"),
    )?;

    let other = create_persona(
        storage,
        CHECK,
        second_owner.identifier,
        &unique_address("bob"),
        false,
    )
    .await?;
    let taken = storage
        .update_persona(
            other.identifier,
            &PersonaUpdate {
                name: other.name.clone(),
                address: address.clone(),
                script: other.script.clone(),
                enabled: false,
            },
            200,
        )
        .await;
    verify(
        CHECK,
        matches!(taken, Err(StorageError::DuplicatePersonaAddress)),
        format!("updating to a used address must fail, got {taken:?}"),
    )?;
    let updated = unwrap_storage(
        CHECK,
        "update_persona",
        storage
            .update_persona(
                persona.identifier,
                &PersonaUpdate {
                    name: "Renamed".into(),
                    address: address.clone(),
                    script: "wait 1s;".into(),
                    enabled: false,
                },
                300,
            )
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "update_persona lost the Persona"))?;
    verify_equal(CHECK, "updated revision", updated.revision, 2)?;
    verify_equal(CHECK, "updated name", updated.name.as_str(), "Renamed")?;
    verify(
        CHECK,
        !updated.enabled,
        "the update must disable the Persona",
    )?;
    verify_equal(CHECK, "created_at", updated.created_at, 100)?;
    verify_equal(CHECK, "updated_at", updated.updated_at, 300)?;
    verify_equal(
        CHECK,
        "updating an unknown Persona",
        unwrap_storage(
            CHECK,
            "update_persona",
            storage
                .update_persona(
                    PersonaIdentifier::new(),
                    &PersonaUpdate {
                        name: "x".into(),
                        address: unique_address("missing"),
                        script: String::new(),
                        enabled: true,
                    },
                    1,
                )
                .await,
        )?,
        None,
    )?;

    let owned = unwrap_storage(
        CHECK,
        "list_personas",
        storage.list_personas(Some(first_owner.identifier)).await,
    )?;
    verify_equal(CHECK, "owned Personas", owned, vec![updated.clone()])?;
    let all = unwrap_storage(CHECK, "list_personas", storage.list_personas(None).await)?;
    verify(
        CHECK,
        all.iter()
            .any(|candidate| candidate.identifier == other.identifier)
            && all
                .iter()
                .any(|candidate| candidate.identifier == updated.identifier),
        "listing every Persona must include both owners",
    )?;

    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "delete_persona",
            storage.delete_persona(other.identifier).await,
        )?,
        "deleting an existing Persona must report it",
    )?;
    verify(
        CHECK,
        !unwrap_storage(
            CHECK,
            "delete_persona",
            storage.delete_persona(other.identifier).await,
        )?,
        "deleting it again must report nothing",
    )?;
    unwrap_storage(
        CHECK,
        "delete_user",
        storage.delete_user(first_owner.identifier).await,
    )?;
    verify_equal(
        CHECK,
        "Persona after owner deletion",
        unwrap_storage(
            CHECK,
            "get_persona",
            storage.get_persona(persona.identifier).await,
        )?,
        None,
    )?;
    Ok(())
}

/**
 * Verify message insertion schedules executions for envelope recipients only.
 *
 * Matching ignores ASCII case, schedules at most one execution per Persona and message even for
 * repeated recipients, ignores MIME-only recipients and disabled Personas, honors the pending
 * limit, and executions disappear with their message or Persona.
 */
pub async fn persona_execution_scheduling(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "persona_execution_scheduling";
    let owner = create_user(
        storage,
        CHECK,
        GlobalRole::Member,
        &unique_email("scheduler"),
    )
    .await?;
    let alice_address = unique_address("alice");
    let bob_address = unique_address("bob");
    let alice = create_persona(storage, CHECK, owner.identifier, &alice_address, true).await?;
    let bob = create_persona(storage, CHECK, owner.identifier, &bob_address, true).await?;
    let disabled_address = unique_address("disabled");
    let disabled =
        create_persona(storage, CHECK, owner.identifier, &disabled_address, false).await?;

    let shouted = alice_address.to_ascii_uppercase();
    let message = deliver(
        storage,
        CHECK,
        &[&shouted, &alice_address, &bob_address, &disabled_address],
    )
    .await?;
    let alice_history = history(storage, CHECK, alice.identifier, owner.identifier).await?;
    verify_equal(CHECK, "Alice executions", alice_history.len(), 1)?;
    let execution = &alice_history[0];
    verify_equal(CHECK, "message", execution.message_identifier, message)?;
    verify_equal(
        CHECK,
        "status",
        execution.status,
        PersonaExecutionStatus::Pending,
    )?;
    verify_equal(CHECK, "scheduled_at", execution.scheduled_at, 500)?;
    verify_equal(CHECK, "started_at", execution.started_at, None)?;
    verify_equal(
        CHECK,
        "Bob executions",
        history(storage, CHECK, bob.identifier, owner.identifier)
            .await?
            .len(),
        1,
    )?;
    verify_equal(
        CHECK,
        "disabled Persona executions",
        history(storage, CHECK, disabled.identifier, owner.identifier)
            .await?
            .len(),
        0,
    )?;

    let mut mime_only = simple_message("mime only");
    mime_only.facts.to = vec![mailbox(&alice_address)];
    mime_only.facts.carbon_copy = vec![mailbox(&alice_address)];
    unwrap_storage(
        CHECK,
        "insert_message",
        storage.insert_message(&mime_only).await,
    )?;
    verify_equal(
        CHECK,
        "executions after a MIME-only recipient",
        history(storage, CHECK, alice.identifier, owner.identifier)
            .await?
            .len(),
        1,
    )?;

    let pending = unwrap_storage(
        CHECK,
        "list_persona_executions_in_status",
        storage
            .list_persona_executions_in_status(PersonaExecutionStatus::Pending, None, 10_000)
            .await,
    )?;
    verify(
        CHECK,
        pending
            .iter()
            .any(|candidate| candidate.identifier == execution.identifier)
            && pending
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence),
        "pending executions must be listed in scheduling order",
    )?;
    let after = unwrap_storage(
        CHECK,
        "list_persona_executions_in_status",
        storage
            .list_persona_executions_in_status(
                PersonaExecutionStatus::Pending,
                Some(execution.sequence),
                10_000,
            )
            .await,
    )?;
    verify(
        CHECK,
        after
            .iter()
            .all(|candidate| candidate.sequence > execution.sequence),
        "the cursor must exclude earlier executions",
    )?;

    unwrap_storage(
        CHECK,
        "delete_message",
        storage.delete_message(message).await,
    )?;
    verify_equal(
        CHECK,
        "executions after message deletion",
        history(storage, CHECK, alice.identifier, owner.identifier)
            .await?
            .len(),
        0,
    )?;

    let capped_address = unique_address("capped");
    let capped = create_persona(storage, CHECK, owner.identifier, &capped_address, true).await?;
    for _ in 0..=PERSONA_PENDING_EXECUTION_LIMIT {
        deliver(storage, CHECK, &[&capped_address]).await?;
    }
    let mut scheduled = 0;
    let mut cursor = None;
    loop {
        let page = unwrap_storage(
            CHECK,
            "list_persona_executions",
            storage
                .list_persona_executions(
                    capped.identifier,
                    owner.identifier,
                    &Expression::True,
                    cursor,
                    100,
                )
                .await,
        )?;
        scheduled += page.len();
        match page.last() {
            Some(last) if page.len() == 100 => cursor = Some(last.execution.sequence),
            _ => break,
        }
    }
    verify_equal(
        CHECK,
        "executions scheduled beyond the pending limit",
        scheduled,
        PERSONA_PENDING_EXECUTION_LIMIT,
    )?;
    unwrap_storage(
        CHECK,
        "delete_persona",
        storage.delete_persona(capped.identifier).await,
    )?;
    verify_equal(
        CHECK,
        "executions after Persona deletion",
        history(storage, CHECK, capped.identifier, owner.identifier)
            .await?
            .len(),
        0,
    )?;
    Ok(())
}

/**
 * Build a recorded step.
 */
fn action(
    step: u32,
    status: PersonaActionStatus,
    detail: PersonaActionDetail,
) -> PersonaExecutionAction {
    PersonaExecutionAction {
        step,
        status,
        line: step + 1,
        column: 1,
        started_at: 600,
        duration_milliseconds: (status != PersonaActionStatus::Started).then_some(25),
        error_message: None,
        detail,
    }
}

/**
 * Verify starting, resuming, recording, finishing, cancelling, discarding, authorized history
 * and retention of executions.
 */
pub async fn persona_execution_lifecycle(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "persona_execution_lifecycle";
    let owner = create_user(
        storage,
        CHECK,
        GlobalRole::Member,
        &unique_email("lifecycle"),
    )
    .await?;
    let address = unique_address("carol");
    let persona = create_persona(storage, CHECK, owner.identifier, &address, true).await?;
    let first_message = deliver(storage, CHECK, &[&address]).await?;
    let second_message = deliver(storage, CHECK, &[&address]).await?;
    let executions = history(storage, CHECK, persona.identifier, owner.identifier).await?;
    verify_equal(CHECK, "scheduled executions", executions.len(), 2)?;
    verify_equal(
        CHECK,
        "newest first",
        executions[0].message_identifier,
        second_message,
    )?;
    let first = executions[1].clone();

    let started = unwrap_storage(
        CHECK,
        "start_persona_execution",
        storage
            .start_persona_execution(first.identifier, 610, persona.revision)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "a pending execution must start"))?;
    verify_equal(
        CHECK,
        "started status",
        started.status,
        PersonaExecutionStatus::Running,
    )?;
    verify_equal(CHECK, "started_at", started.started_at, Some(610))?;
    verify_equal(
        CHECK,
        "revision",
        started.persona_revision,
        Some(persona.revision),
    )?;
    let resumed = unwrap_storage(
        CHECK,
        "start_persona_execution",
        storage
            .start_persona_execution(first.identifier, 999, 77)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "a running execution must be returned for resumption"))?;
    verify_equal(CHECK, "resumed execution", resumed, started.clone())?;

    let open = PersonaActionDetail::Open {
        images: Vec::new(),
        omitted_images: 0,
        embedded_images: 1,
        rejected_images: 0,
    };
    let wait = PersonaActionDetail::Wait { milliseconds: 5000 };
    for recorded in [
        action(1, PersonaActionStatus::Started, wait.clone()),
        action(0, PersonaActionStatus::Succeeded, open.clone()),
    ] {
        unwrap_storage(
            CHECK,
            "record_persona_execution_action",
            storage
                .record_persona_execution_action(first.identifier, &recorded)
                .await,
        )?;
    }
    let missing = storage
        .record_persona_execution_action(
            sandpost_core::PersonaExecutionIdentifier::new(),
            &action(0, PersonaActionStatus::Started, wait.clone()),
        )
        .await;
    verify(
        CHECK,
        matches!(missing, Err(StorageError::NotFound)),
        format!("recording for an unknown execution must be NotFound, got {missing:?}"),
    )?;
    let (_, steps) = unwrap_storage(
        CHECK,
        "get_persona_execution",
        storage.get_persona_execution(first.identifier).await,
    )?
    .ok_or_else(|| failure(CHECK, "the execution must exist"))?;
    verify_equal(
        CHECK,
        "steps in order",
        steps.iter().map(|step| step.step).collect::<Vec<_>>(),
        vec![0, 1],
    )?;
    verify_equal(
        CHECK,
        "round-tripped step",
        steps[0].clone(),
        action(0, PersonaActionStatus::Succeeded, open),
    )?;

    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "finish_persona_execution",
            storage
                .finish_persona_execution(
                    first.identifier,
                    PersonaExecutionStatus::Failed,
                    620,
                    Some("boom"),
                )
                .await,
        )?,
        "a running execution must finish",
    )?;
    verify(
        CHECK,
        !unwrap_storage(
            CHECK,
            "finish_persona_execution",
            storage
                .finish_persona_execution(
                    first.identifier,
                    PersonaExecutionStatus::Completed,
                    630,
                    None,
                )
                .await,
        )?,
        "a terminal execution must not change again",
    )?;
    let (finished, steps) = unwrap_storage(
        CHECK,
        "get_persona_execution",
        storage.get_persona_execution(first.identifier).await,
    )?
    .ok_or_else(|| failure(CHECK, "the execution must exist"))?;
    verify_equal(
        CHECK,
        "finished status",
        finished.status,
        PersonaExecutionStatus::Failed,
    )?;
    verify_equal(CHECK, "completed_at", finished.completed_at, Some(620))?;
    verify_equal(
        CHECK,
        "error",
        finished.error_message.as_deref(),
        Some("boom"),
    )?;
    verify_equal(
        CHECK,
        "unfinished step",
        steps[1].status,
        PersonaActionStatus::Failed,
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "start_persona_execution",
            storage
                .start_persona_execution(first.identifier, 640, 1)
                .await,
        )?
        .is_none(),
        "a terminal execution must not start",
    )?;
    let not_terminal = storage
        .finish_persona_execution(
            executions[0].identifier,
            PersonaExecutionStatus::Running,
            1,
            None,
        )
        .await;
    verify(
        CHECK,
        matches!(not_terminal, Err(StorageError::InvalidData(_))),
        "finishing with a non-terminal status must be refused",
    )?;

    verify_equal(
        CHECK,
        "cancelled executions",
        unwrap_storage(
            CHECK,
            "cancel_persona_executions",
            storage
                .cancel_persona_executions(persona.identifier, "persona disabled", 650)
                .await,
        )?,
        1,
    )?;
    let (cancelled, _) = unwrap_storage(
        CHECK,
        "get_persona_execution",
        storage
            .get_persona_execution(executions[0].identifier)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "the execution must exist"))?;
    verify_equal(
        CHECK,
        "cancelled status",
        cancelled.status,
        PersonaExecutionStatus::Cancelled,
    )?;
    verify_equal(
        CHECK,
        "cancel reason",
        cancelled.error_message.as_deref(),
        Some("persona disabled"),
    )?;

    let subject_filter = Expression::Predicate(Predicate {
        field: Field::Subject,
        operator: Operator::Equal,
        value: Value::String("never matches".into()),
    });
    verify_equal(
        CHECK,
        "history under an excluding filter",
        unwrap_storage(
            CHECK,
            "list_persona_executions",
            storage
                .list_persona_executions(
                    persona.identifier,
                    owner.identifier,
                    &subject_filter,
                    None,
                    10,
                )
                .await,
        )?
        .len(),
        0,
    )?;
    unwrap_storage(
        CHECK,
        "delete_user_messages",
        storage
            .delete_user_messages(owner.identifier, Some(&[first_message]), &Expression::True)
            .await,
    )?;
    let visible = history(storage, CHECK, persona.identifier, owner.identifier).await?;
    verify_equal(
        CHECK,
        "history after the viewer deleted a message",
        visible
            .iter()
            .map(|execution| execution.message_identifier)
            .collect::<Vec<_>>(),
        vec![second_message],
    )?;
    let entries = unwrap_storage(
        CHECK,
        "list_persona_executions",
        storage
            .list_persona_executions(
                persona.identifier,
                owner.identifier,
                &Expression::True,
                None,
                10,
            )
            .await,
    )?;
    verify_equal(
        CHECK,
        "history entry subject",
        entries.first().map(|entry| entry.subject.as_str()),
        Some("persona delivery"),
    )?;
    let page = unwrap_storage(
        CHECK,
        "list_persona_executions",
        storage
            .list_persona_executions(
                persona.identifier,
                owner.identifier,
                &Expression::True,
                Some(visible[0].sequence),
                10,
            )
            .await,
    )?;
    verify_equal(CHECK, "page before the newest", page.len(), 0)?;

    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "discard_persona_execution",
            storage
                .discard_persona_execution(executions[0].identifier)
                .await,
        )?,
        "discarding an execution must report it",
    )?;
    verify_equal(
        CHECK,
        "discarded execution",
        unwrap_storage(
            CHECK,
            "get_persona_execution",
            storage
                .get_persona_execution(executions[0].identifier)
                .await,
        )?,
        None,
    )?;

    let retained_address = unique_address("retained");
    let retained =
        create_persona(storage, CHECK, owner.identifier, &retained_address, true).await?;
    for _ in 0..PERSONA_EXECUTION_RETENTION + 2 {
        deliver(storage, CHECK, &[&retained_address]).await?;
    }
    let mut cursor = None;
    loop {
        let pending = unwrap_storage(
            CHECK,
            "list_persona_executions_in_status",
            storage
                .list_persona_executions_in_status(PersonaExecutionStatus::Pending, cursor, 256)
                .await,
        )?;
        let Some(last) = pending.last() else {
            break;
        };
        cursor = Some(last.sequence);
        for execution in pending
            .iter()
            .filter(|execution| execution.persona_identifier == retained.identifier)
        {
            unwrap_storage(
                CHECK,
                "finish_persona_execution",
                storage
                    .finish_persona_execution(
                        execution.identifier,
                        PersonaExecutionStatus::Completed,
                        700,
                        None,
                    )
                    .await,
            )?;
        }
    }
    let mut remaining = 0;
    let mut oldest = u64::MAX;
    let mut cursor = None;
    loop {
        let page = unwrap_storage(
            CHECK,
            "list_persona_executions",
            storage
                .list_persona_executions(
                    retained.identifier,
                    owner.identifier,
                    &Expression::True,
                    cursor,
                    100,
                )
                .await,
        )?;
        remaining += page.len();
        if let Some(last) = page.last() {
            oldest = oldest.min(last.execution.sequence);
        }
        match page.last() {
            Some(last) if page.len() == 100 => cursor = Some(last.execution.sequence),
            _ => break,
        }
    }
    verify_equal(
        CHECK,
        "retained executions",
        remaining,
        PERSONA_EXECUTION_RETENTION,
    )?;
    verify(
        CHECK,
        oldest != u64::MAX,
        "retained history must not be empty",
    )?;
    Ok(())
}
