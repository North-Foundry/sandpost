//! Endpoint and endpoint-membership conformance checks.
use sandpost_core::{
    EndpointIdentifier, EndpointMembership, EndpointRole, GlobalRole, MailAccess, SmtpEndpoint,
    UserIdentifier,
};
use sandpost_storage::Storage;

use crate::ConformanceFailure;
use crate::support::{
    create_user, failure, save_endpoint, unique_email, unwrap_storage, verify, verify_equal,
};

/// Verify endpoint create, read, update, and delete behaviour.
pub async fn endpoint_crud(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "endpoint_crud";
    let identifier = EndpointIdentifier::new();
    let created = SmtpEndpoint {
        identifier,
        name: "conformance endpoint".to_owned(),
    };
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "get_endpoint before save",
            storage.get_endpoint(identifier).await,
        )?
        .is_none(),
        "get_endpoint must return none before the endpoint is saved",
    )?;
    unwrap_storage(
        CHECK,
        "save_endpoint",
        storage.save_endpoint(&created).await,
    )?;
    verify_equal(
        CHECK,
        "get_endpoint",
        unwrap_storage(
            CHECK,
            "get_endpoint after save",
            storage.get_endpoint(identifier).await,
        )?
        .ok_or_else(|| failure(CHECK, "get_endpoint returned none for a saved endpoint"))?,
        created.clone(),
    )?;
    let listed = unwrap_storage(CHECK, "list_endpoints", storage.list_endpoints().await)?;
    verify(
        CHECK,
        listed.contains(&created),
        "list_endpoints omitted a saved endpoint",
    )?;

    let renamed = SmtpEndpoint {
        identifier,
        name: "renamed endpoint".to_owned(),
    };
    unwrap_storage(
        CHECK,
        "save_endpoint update",
        storage.save_endpoint(&renamed).await,
    )?;
    verify_equal(
        CHECK,
        "renamed endpoint",
        unwrap_storage(
            CHECK,
            "get_endpoint after rename",
            storage.get_endpoint(identifier).await,
        )?
        .ok_or_else(|| failure(CHECK, "renamed endpoint disappeared"))?,
        renamed,
    )?;

    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "delete_endpoint",
            storage.delete_endpoint(identifier).await,
        )?,
        "delete_endpoint must report true for an existing endpoint",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "get_endpoint after delete",
            storage.get_endpoint(identifier).await,
        )?
        .is_none(),
        "a deleted endpoint must not be readable",
    )?;
    verify(
        CHECK,
        !unwrap_storage(
            CHECK,
            "delete_endpoint twice",
            storage.delete_endpoint(identifier).await,
        )?,
        "delete_endpoint must report false for a missing endpoint",
    )?;
    Ok(())
}

/// Verify endpoint memberships, roles, mail access, and removal.
pub async fn endpoint_memberships(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "endpoint_memberships";
    let first = save_endpoint(storage, CHECK, "membership first endpoint").await?;
    let second = save_endpoint(storage, CHECK, "membership second endpoint").await?;
    let first_user =
        create_user(storage, CHECK, GlobalRole::Member, &unique_email("member")).await?;
    let second_user =
        create_user(storage, CHECK, GlobalRole::Member, &unique_email("member")).await?;
    let third_user =
        create_user(storage, CHECK, GlobalRole::Member, &unique_email("member")).await?;
    let fourth_user =
        create_user(storage, CHECK, GlobalRole::Member, &unique_email("member")).await?;

    let member_all = EndpointMembership {
        user_identifier: first_user.identifier,
        endpoint_identifier: first,
        role: EndpointRole::Member,
        mail_access: MailAccess::All,
    };
    unwrap_storage(
        CHECK,
        "set_endpoint_membership",
        storage.set_endpoint_membership(&member_all).await,
    )?;
    verify_equal(
        CHECK,
        "membership read",
        unwrap_storage(
            CHECK,
            "get_endpoint_membership",
            storage
                .get_endpoint_membership(first_user.identifier, first)
                .await,
        )?
        .ok_or_else(|| failure(CHECK, "get_endpoint_membership returned none"))?,
        member_all.clone(),
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "list_endpoint_memberships",
            storage.list_endpoint_memberships(first).await,
        )?
        .contains(&member_all),
        "list_endpoint_memberships omitted a membership",
    )?;
    verify_equal(
        CHECK,
        "list_user_endpoint_memberships",
        unwrap_storage(
            CHECK,
            "list_user_endpoint_memberships",
            storage
                .list_user_endpoint_memberships(first_user.identifier)
                .await,
        )?,
        vec![member_all.clone()],
    )?;

    let viewer_scoped = EndpointMembership {
        role: EndpointRole::Viewer,
        mail_access: MailAccess::Scoped,
        ..member_all.clone()
    };
    unwrap_storage(
        CHECK,
        "upsert endpoint membership",
        storage.set_endpoint_membership(&viewer_scoped).await,
    )?;
    verify_equal(
        CHECK,
        "upserted membership",
        unwrap_storage(
            CHECK,
            "get_endpoint_membership after upsert",
            storage
                .get_endpoint_membership(first_user.identifier, first)
                .await,
        )?
        .ok_or_else(|| failure(CHECK, "upserted membership disappeared"))?,
        viewer_scoped,
    )?;

    for (user, role, mail_access) in [
        (
            second_user.identifier,
            EndpointRole::Member,
            MailAccess::Scoped,
        ),
        (third_user.identifier, EndpointRole::Admin, MailAccess::All),
        (
            fourth_user.identifier,
            EndpointRole::Viewer,
            MailAccess::All,
        ),
    ] {
        unwrap_storage(
            CHECK,
            "set additional membership",
            storage
                .set_endpoint_membership(&EndpointMembership {
                    user_identifier: user,
                    endpoint_identifier: first,
                    role,
                    mail_access,
                })
                .await,
        )?;
    }
    let endpoint_members = unwrap_storage(
        CHECK,
        "list_endpoint_memberships",
        storage.list_endpoint_memberships(first).await,
    )?;
    verify_equal(
        CHECK,
        "endpoint membership count",
        endpoint_members.len(),
        4,
    )?;
    verify(
        CHECK,
        endpoint_members
            .windows(2)
            .all(|window| window[0].user_identifier < window[1].user_identifier),
        "list_endpoint_memberships must be ordered by user identifier",
    )?;

    let second_membership = EndpointMembership {
        user_identifier: first_user.identifier,
        endpoint_identifier: second,
        role: EndpointRole::Admin,
        mail_access: MailAccess::All,
    };
    unwrap_storage(
        CHECK,
        "set membership on a second endpoint",
        storage.set_endpoint_membership(&second_membership).await,
    )?;
    let user_memberships = unwrap_storage(
        CHECK,
        "list_user_endpoint_memberships",
        storage
            .list_user_endpoint_memberships(first_user.identifier)
            .await,
    )?;
    verify_equal(CHECK, "user membership count", user_memberships.len(), 2)?;
    let ordered_endpoints: Vec<EndpointIdentifier> = user_memberships
        .iter()
        .map(|membership| membership.endpoint_identifier)
        .collect();
    let expected_endpoints = {
        let mut endpoints = vec![first, second];
        endpoints.sort();
        endpoints
    };
    verify_equal(
        CHECK,
        "user membership endpoints",
        ordered_endpoints,
        expected_endpoints,
    )?;
    verify(
        CHECK,
        user_memberships.iter().any(|membership| {
            membership.role == EndpointRole::Viewer && membership.endpoint_identifier == first
        }) && user_memberships.iter().any(|membership| {
            membership.role == EndpointRole::Admin && membership.endpoint_identifier == second
        }),
        "one user must be able to hold different roles on different endpoints",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "get_endpoint_membership for an unknown user",
            storage
                .get_endpoint_membership(UserIdentifier::new(), first)
                .await,
        )?
        .is_none(),
        "an unknown user must have no endpoint membership",
    )?;

    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "remove_endpoint_membership",
            storage
                .remove_endpoint_membership(first_user.identifier, second)
                .await,
        )?,
        "remove_endpoint_membership must report true for an existing membership",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "get_endpoint_membership after remove",
            storage
                .get_endpoint_membership(first_user.identifier, second)
                .await,
        )?
        .is_none(),
        "a removed membership must be gone",
    )?;
    verify(
        CHECK,
        !unwrap_storage(
            CHECK,
            "remove_endpoint_membership twice",
            storage
                .remove_endpoint_membership(first_user.identifier, second)
                .await,
        )?,
        "remove_endpoint_membership must report false when absent",
    )?;

    verify_equal(
        CHECK,
        "scoped mail access",
        unwrap_storage(
            CHECK,
            "get_endpoint_membership for scoped member",
            storage
                .get_endpoint_membership(second_user.identifier, first)
                .await,
        )?
        .ok_or_else(|| failure(CHECK, "scoped membership disappeared"))?
        .mail_access,
        MailAccess::Scoped,
    )?;
    verify_equal(
        CHECK,
        "all mail access",
        unwrap_storage(
            CHECK,
            "get_endpoint_membership for all-mail member",
            storage
                .get_endpoint_membership(third_user.identifier, first)
                .await,
        )?
        .ok_or_else(|| failure(CHECK, "all-mail membership disappeared"))?
        .mail_access,
        MailAccess::All,
    )?;
    Ok(())
}
