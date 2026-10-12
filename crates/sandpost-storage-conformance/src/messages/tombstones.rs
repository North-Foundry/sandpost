//! Per-user deletion behavior checks.
use sandpost_core::{GlobalRole, MessageIdentifier};
use sandpost_query::Expression;
use sandpost_storage::{MessageListQuery, Storage};

use crate::ConformanceFailure;
use crate::support::{
    create_user, simple_message, unique_email, unwrap_storage, verify, verify_equal,
};

/// Verify user deletions are persistent state and leave shared mail visible to other users.
pub async fn user_message_deletions(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "user_message_deletions";
    let deleting_user = create_user(
        storage,
        CHECK,
        GlobalRole::Member,
        &unique_email("tombstone-owner"),
    )
    .await?;
    let other_user = create_user(
        storage,
        CHECK,
        GlobalRole::Member,
        &unique_email("tombstone-other"),
    )
    .await?;
    let mut specifically_deleted = simple_message("specific tombstone target");
    specifically_deleted.raw_message = b"preserved raw content".to_vec();
    let deleted_identifier = specifically_deleted.identifier;
    unwrap_storage(
        CHECK,
        "insert target message",
        storage.insert_message(&specifically_deleted).await,
    )?;
    let bulk_message = simple_message("bulk tombstone match");
    let bulk_identifier = bulk_message.identifier;
    unwrap_storage(
        CHECK,
        "insert bulk message",
        storage.insert_message(&bulk_message).await,
    )?;
    let retained_message = simple_message("bulk tombstone miss");
    let retained_identifier = retained_message.identifier;
    unwrap_storage(
        CHECK,
        "insert retained message",
        storage.insert_message(&retained_message).await,
    )?;

    verify_equal(
        CHECK,
        "first bounded deletion count",
        unwrap_storage(
            CHECK,
            "delete one user's message",
            storage
                .delete_user_messages(
                    deleting_user.identifier,
                    Some(&[deleted_identifier]),
                    &Expression::True,
                )
                .await,
        )?,
        1,
    )?;
    verify_equal(
        CHECK,
        "repeated deletion count",
        unwrap_storage(
            CHECK,
            "repeat one user's deletion",
            storage
                .delete_user_messages(
                    deleting_user.identifier,
                    Some(&[deleted_identifier]),
                    &Expression::True,
                )
                .await,
        )?,
        0,
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "read tombstoned user's metadata",
            storage
                .get_user_message_metadata(deleting_user.identifier, deleted_identifier)
                .await,
        )?
        .is_none(),
        "metadata reads must use the same per-user deletion state",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "read tombstoned user's message",
            storage
                .get_user_message(deleting_user.identifier, deleted_identifier)
                .await,
        )?
        .is_none(),
        "the deleting user must not read their tombstoned message",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "read shared message as another user",
            storage
                .get_user_message(other_user.identifier, deleted_identifier)
                .await,
        )?
        .is_some(),
        "a user's tombstone must not hide shared mail from another user",
    )?;

    let bulk_filter = sandpost_query::Expression::Predicate(sandpost_query::Predicate {
        field: sandpost_query::Field::Subject,
        operator: sandpost_query::Operator::Equal,
        value: sandpost_query::Value::String("bulk tombstone match".to_owned()),
    });
    verify_equal(
        CHECK,
        "filter deletion count",
        unwrap_storage(
            CHECK,
            "delete all filtered mail for one user",
            storage
                .delete_user_messages(deleting_user.identifier, None, &bulk_filter)
                .await,
        )?,
        1,
    )?;
    let indexed = unwrap_storage(
        CHECK,
        "load current revisions",
        storage
            .indexed_messages(&[deleted_identifier, bulk_identifier, retained_identifier])
            .await,
    )?;
    let requested = indexed
        .iter()
        .map(|message| (message.identifier, message.revision))
        .collect::<Vec<_>>();
    let hydrated = unwrap_storage(
        CHECK,
        "hydrate current user messages without tombstones",
        storage
            .hydrate_current_user_messages(deleting_user.identifier, &requested, &Expression::True)
            .await,
    )?;
    verify_equal(
        CHECK,
        "visible hydrated identifiers",
        hydrated
            .iter()
            .map(|summary| summary.identifier)
            .collect::<Vec<MessageIdentifier>>(),
        vec![retained_identifier],
    )?;
    let filter_rejected = unwrap_storage(
        CHECK,
        "apply authorization filter during hydration",
        storage
            .hydrate_current_user_messages(
                deleting_user.identifier,
                &requested,
                &sandpost_query::Expression::Predicate(sandpost_query::Predicate {
                    field: sandpost_query::Field::Subject,
                    operator: sandpost_query::Operator::Equal,
                    value: sandpost_query::Value::String("not a fixture subject".to_owned()),
                }),
            )
            .await,
    )?;
    verify(
        CHECK,
        filter_rejected.is_empty(),
        "hydration must omit rows rejected by the supplied authorization filter",
    )?;
    let listed = unwrap_storage(
        CHECK,
        "list user messages",
        storage
            .list_user_messages(
                deleting_user.identifier,
                MessageListQuery {
                    filter: None,
                    before: None,
                    limit: usize::MAX,
                },
            )
            .await,
    )?;
    verify(
        CHECK,
        listed
            .iter()
            .any(|summary| summary.identifier == retained_identifier)
            && listed.iter().all(|summary| {
                summary.identifier != deleted_identifier && summary.identifier != bulk_identifier
            }),
        "user listing must exclude only this user's tombstoned messages",
    )?;
    let physical_messages = unwrap_storage(
        CHECK,
        "confirm tombstones preserve shared physical mail",
        storage
            .indexed_messages(&[deleted_identifier, bulk_identifier, retained_identifier])
            .await,
    )?;
    verify_equal(
        CHECK,
        "physical index records after tombstones",
        physical_messages
            .iter()
            .map(|message| message.identifier)
            .collect::<Vec<_>>(),
        vec![deleted_identifier, bulk_identifier, retained_identifier],
    )?;
    for identifier in [deleted_identifier, bulk_identifier, retained_identifier] {
        verify(
            CHECK,
            unwrap_storage(
                CHECK,
                "read physical message after tombstone",
                storage.get_message(identifier).await,
            )?
            .is_some(),
            "user deletion must preserve physical message contents",
        )?;
    }
    Ok(())
}
