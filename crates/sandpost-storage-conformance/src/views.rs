//! Saved-view conformance checks.
use sandpost_core::{GlobalRole, View, ViewIdentifier};
use sandpost_storage::Storage;

use crate::ConformanceFailure;
use crate::support::{
    create_user, failure, save_endpoint, unique_email, unwrap_storage, verify, verify_equal,
};

/// Verify personal and shared view create, read, update, and delete behaviour.
pub async fn view_crud(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "view_crud";
    let endpoint = save_endpoint(storage, CHECK, "view endpoint").await?;
    let owner = create_user(
        storage,
        CHECK,
        GlobalRole::Member,
        &unique_email("view-owner"),
    )
    .await?;
    let other = create_user(
        storage,
        CHECK,
        GlobalRole::Member,
        &unique_email("view-other"),
    )
    .await?;

    let personal = View {
        identifier: ViewIdentifier::new(),
        endpoint_identifier: endpoint,
        owner_identifier: Some(owner.identifier),
        name: "aaa personal view".to_owned(),
        filter: "subject == \"personal\"".to_owned(),
    };
    let shared = View {
        identifier: ViewIdentifier::new(),
        endpoint_identifier: endpoint,
        owner_identifier: None,
        name: "zzz shared view".to_owned(),
        filter: "subject == \"shared\"".to_owned(),
    };
    unwrap_storage(
        CHECK,
        "save personal view",
        storage.save_view(&personal).await,
    )?;
    unwrap_storage(CHECK, "save shared view", storage.save_view(&shared).await)?;
    verify_equal(
        CHECK,
        "get personal view",
        unwrap_storage(
            CHECK,
            "get_view personal",
            storage.get_view(personal.identifier).await,
        )?
        .ok_or_else(|| failure(CHECK, "get_view returned none for a personal view"))?,
        personal.clone(),
    )?;
    verify_equal(
        CHECK,
        "get shared view",
        unwrap_storage(
            CHECK,
            "get_view shared",
            storage.get_view(shared.identifier).await,
        )?
        .ok_or_else(|| failure(CHECK, "get_view returned none for a shared view"))?,
        shared.clone(),
    )?;

    let owner_views = unwrap_storage(
        CHECK,
        "list_views owner",
        storage.list_views(owner.identifier).await,
    )?;
    verify(
        CHECK,
        owner_views.contains(&personal) && owner_views.contains(&shared),
        "the owner must see both personal and shared views",
    )?;
    verify(
        CHECK,
        owner_views.windows(2).all(|window| {
            (window[0].name.as_str(), window[0].identifier)
                <= (window[1].name.as_str(), window[1].identifier)
        }),
        "list_views must be ordered by name then identifier",
    )?;
    let other_views = unwrap_storage(
        CHECK,
        "list_views other",
        storage.list_views(other.identifier).await,
    )?;
    verify(
        CHECK,
        other_views.contains(&shared),
        "a shared view must be visible to every user",
    )?;
    verify(
        CHECK,
        other_views
            .iter()
            .all(|view| view.identifier != personal.identifier),
        "a personal view must not be visible to another user",
    )?;

    let renamed = View {
        name: "renamed personal view".to_owned(),
        ..personal.clone()
    };
    unwrap_storage(
        CHECK,
        "update personal view",
        storage.save_view(&renamed).await,
    )?;
    verify_equal(
        CHECK,
        "renamed view",
        unwrap_storage(
            CHECK,
            "get_view after rename",
            storage.get_view(personal.identifier).await,
        )?
        .ok_or_else(|| failure(CHECK, "renamed view disappeared"))?
        .name,
        "renamed personal view".to_owned(),
    )?;

    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "delete_view",
            storage.delete_view(personal.identifier).await,
        )?,
        "delete_view must report true for an existing view",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "get_view after delete",
            storage.get_view(personal.identifier).await,
        )?
        .is_none(),
        "a deleted view must be gone",
    )?;
    verify(
        CHECK,
        !unwrap_storage(
            CHECK,
            "delete_view twice",
            storage.delete_view(personal.identifier).await,
        )?,
        "delete_view must report false when absent",
    )?;
    Ok(())
}
