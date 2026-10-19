//! HTTP management of users' own IMAP accounts.
use super::*;

/// Send one request with an optional JSON body and return the status, headers, and JSON body.
async fn exchange(
    application: &Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
    let request = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(body) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = application.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            serde_json::Value::String(String::from_utf8_lossy(&bytes).into_owned())
        })
    };
    (status, headers, json)
}

/// Build an owner router and a router for a second user with the given role.
async fn routers(role: GlobalRole) -> (Router, Router) {
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, owner_token) = initialized_state(memory_storage().await, events).await;
    let owner = state.instance().session(&owner_token).await.unwrap();
    state
        .authorized(&owner)
        .create_user(NewAccount {
            name: "Second User".into(),
            email: "second@example.test".into(),
            password: "second-user-password".into(),
            global_role: role,
            mail_access: sandpost_core::MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    let token = state
        .instance()
        .login(LoginRequest {
            remember_me: false,
            email: "second@example.test".into(),
            password: "second-user-password".into(),
        })
        .await
        .unwrap()
        .token;
    (
        with_test_auth(router(state.clone()), owner_token),
        with_test_auth(router(state), token),
    )
}

/// Members manage their own accounts; passwords appear only in no-store issuing responses and
/// hashes never appear.
#[tokio::test]
async fn users_manage_their_own_imap_accounts_without_secret_leaks() {
    let (_, member) = routers(GlobalRole::Member).await;
    let (status, headers, created) = exchange(
        &member,
        "POST",
        "/api/v1/imap/accounts",
        Some(serde_json::json!({ "name": "Laravel Debug" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    let password = created["password"].as_str().unwrap().to_owned();
    assert_eq!(password.len(), 32);
    let account = &created["account"];
    let identifier = account["identifier"].as_str().unwrap().to_owned();
    assert!(
        account["username"]
            .as_str()
            .unwrap()
            .starts_with("laravel-debug-")
    );
    assert_eq!(account["mirror_views"], true);
    assert_eq!(
        account["mailboxes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|mailbox| mailbox["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["INBOX", "Trash"]
    );
    assert_eq!(created["server"]["enabled"], false);
    for uri in [
        "/api/v1/imap/accounts".to_owned(),
        format!("/api/v1/imap/accounts/{identifier}"),
        "/api/v1/imap/server".to_owned(),
    ] {
        let (status, _, body) = exchange(&member, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        let text = body.to_string();
        assert!(!text.contains(&password), "{uri} exposes the password");
        assert!(
            !text.contains("password"),
            "{uri} exposes password material"
        );
        assert!(!text.contains("$argon2"), "{uri} exposes the hash");
    }
    let (status, headers, rotated) = exchange(
        &member,
        "POST",
        &format!("/api/v1/imap/accounts/{identifier}/password"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_ne!(rotated["password"].as_str().unwrap(), password);
    let (status, _, updated) = exchange(
        &member,
        "PUT",
        &format!("/api/v1/imap/accounts/{identifier}"),
        Some(serde_json::json!({ "enabled": false, "name": "Renamed" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["enabled"], false);
    assert_eq!(updated["name"], "Renamed");
    let (status, _, _) = exchange(
        &member,
        "DELETE",
        &format!("/api/v1/imap/accounts/{identifier}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, _, listed) = exchange(&member, "GET", "/api/v1/imap/accounts", None).await;
    assert_eq!(listed, serde_json::json!([]));
}

/// Another user, even the owner of the installation, cannot read or change an account.
#[tokio::test]
async fn accounts_are_private_to_their_owner() {
    let (owner, member) = routers(GlobalRole::Member).await;
    let (_, _, created) = exchange(
        &member,
        "POST",
        "/api/v1/imap/accounts",
        Some(serde_json::json!({ "name": "Private", "username": "private.account" })),
    )
    .await;
    let identifier = created["account"]["identifier"]
        .as_str()
        .unwrap()
        .to_owned();
    for (method, uri, body) in [
        ("GET", format!("/api/v1/imap/accounts/{identifier}"), None),
        (
            "PUT",
            format!("/api/v1/imap/accounts/{identifier}"),
            Some(serde_json::json!({ "enabled": false })),
        ),
        (
            "DELETE",
            format!("/api/v1/imap/accounts/{identifier}"),
            None,
        ),
        (
            "POST",
            format!("/api/v1/imap/accounts/{identifier}/password"),
            None,
        ),
        (
            "POST",
            format!("/api/v1/imap/accounts/{identifier}/folders"),
            Some(serde_json::json!({ "name": "x", "filter": "true" })),
        ),
    ] {
        let (status, _, _) = exchange(&owner, method, &uri, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
    }
    let (_, _, owner_list) = exchange(&owner, "GET", "/api/v1/imap/accounts", None).await;
    assert_eq!(owner_list, serde_json::json!([]));
    let (status, _, _) = exchange(
        &owner,
        "POST",
        "/api/v1/imap/accounts",
        Some(serde_json::json!({ "name": "Taken", "username": "private.account" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "usernames are unique across users"
    );
}

/// Local folders are validated, created, renamed, and deleted; invalid filters are refused.
#[tokio::test]
async fn local_folders_validate_filters() {
    let (owner, _) = routers(GlobalRole::Member).await;
    let (_, _, created) = exchange(
        &owner,
        "POST",
        "/api/v1/imap/accounts",
        Some(serde_json::json!({ "name": "Folders", "mirror_views": false })),
    )
    .await;
    let identifier = created["account"]["identifier"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, _, refused) = exchange(
        &owner,
        "POST",
        &format!("/api/v1/imap/accounts/{identifier}/folders"),
        Some(serde_json::json!({ "name": "Broken", "filter": "subject ===" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let (status, _, folder) = exchange(
        &owner,
        "POST",
        &format!("/api/v1/imap/accounts/{identifier}/folders"),
        Some(serde_json::json!({
            "name": "Failed Payments",
            "filter": "subject contains \"failed\"",
            "include_in_inbox": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let folder_identifier = folder["identifier"].as_str().unwrap().to_owned();
    let (status, _, duplicate) = exchange(
        &owner,
        "POST",
        &format!("/api/v1/imap/accounts/{identifier}/folders"),
        Some(serde_json::json!({ "name": "Failed Payments", "filter": "true" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{duplicate}");
    let (status, _, renamed) = exchange(
        &owner,
        "PUT",
        &format!("/api/v1/imap/accounts/{identifier}/folders/{folder_identifier}"),
        Some(serde_json::json!({ "name": "Payments", "filter": "subject contains \"payment\"" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["include_in_inbox"], true);
    let (_, _, account) = exchange(
        &owner,
        "GET",
        &format!("/api/v1/imap/accounts/{identifier}"),
        None,
    )
    .await;
    assert_eq!(account["mailboxes"][2]["name"], "Payments");
    assert_eq!(account["mailboxes"][2]["source"], "folder");
    let (status, _, _) = exchange(
        &owner,
        "DELETE",
        &format!("/api/v1/imap/accounts/{identifier}/folders/{folder_identifier}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, restored) = exchange(
        &owner,
        "POST",
        &format!("/api/v1/imap/accounts/{identifier}/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(restored["restored"], 0);
}

/// IMAP management requires a session.
#[tokio::test]
async fn imap_routes_require_authentication() {
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, _) = initialized_state(memory_storage().await, events).await;
    let application = router(state);
    for uri in ["/api/v1/imap/accounts", "/api/v1/imap/server"] {
        let (status, _, _) = exchange(&application, "GET", uri, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
    }
}
