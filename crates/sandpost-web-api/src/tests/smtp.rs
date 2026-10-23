//! HTTP administration of the global SMTP server and SMTP access credentials.
use super::*;

/// Send one request with an optional JSON body and return the status, headers, and JSON body.
pub(super) async fn exchange(
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

/// Create a non-administrator account and return a router authenticated as it.
pub(super) async fn member_router(
    state: &ApplicationState,
    owner_token: &str,
    email: &str,
    role: GlobalRole,
) -> Router {
    let owner = state.instance().session(owner_token).await.unwrap();
    state
        .authorized(&owner)
        .create_user(NewAccount {
            name: "Restricted Member".into(),
            email: email.into(),
            password: "member-user-password".into(),
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
            email: email.into(),
            password: "member-user-password".into(),
        })
        .await
        .unwrap()
        .token;
    with_test_auth(router(state.clone()), token)
}

/// Administrators manage accesses; the password appears only in no-store issuing responses.
#[tokio::test]
async fn administrators_manage_smtp_accesses_and_read_no_secrets() {
    let application = initialized_router(memory_storage().await).await;
    let (status, headers, created) = exchange(
        &application,
        "POST",
        "/api/v1/smtp/accesses",
        Some(serde_json::json!({ "name": "Laravel Development" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    let password = created["password"].as_str().unwrap().to_owned();
    assert_eq!(password.len(), 32);
    let identifier = created["access"]["identifier"].as_str().unwrap().to_owned();
    assert!(
        created["access"]["username"]
            .as_str()
            .unwrap()
            .starts_with("laravel-development-")
    );
    assert!(created["server"]["listen_address"].is_null());
    assert!(created["access"].get("password_hash").is_none());

    for uri in ["/api/v1/smtp/accesses", "/api/v1/smtp/server"] {
        let (status, _, body) = exchange(&application, "GET", uri, None).await;
        assert_eq!(status, StatusCode::OK);
        let text = body.to_string();
        assert!(
            !text.contains(&password),
            "{uri} must not expose the password"
        );
        assert!(
            !text.contains("password"),
            "{uri} must not expose password material"
        );
        assert!(!text.contains("$argon2"), "{uri} must not expose the hash");
    }

    let (status, headers, regenerated) = exchange(
        &application,
        "POST",
        &format!("/api/v1/smtp/accesses/{identifier}/password"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_ne!(regenerated["password"].as_str().unwrap(), password);

    let (status, _, updated) = exchange(
        &application,
        "PUT",
        &format!("/api/v1/smtp/accesses/{identifier}"),
        Some(serde_json::json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["enabled"], false);
    assert!(updated.get("password").is_none());

    let (status, _, _) = exchange(
        &application,
        "DELETE",
        &format!("/api/v1/smtp/accesses/{identifier}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, listed) = exchange(&application, "GET", "/api/v1/smtp/accesses", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed, serde_json::json!([]));
    let (status, _, _) = exchange(
        &application,
        "DELETE",
        &format!("/api/v1/smtp/accesses/{identifier}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Members and viewers receive 403 from every SMTP administration route.
#[tokio::test]
async fn members_and_viewers_cannot_use_smtp_administration_routes() {
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, owner_token) = initialized_state(memory_storage().await, events).await;
    let owner = with_test_auth(router(state.clone()), owner_token.clone());
    let (_, _, created) = exchange(
        &owner,
        "POST",
        "/api/v1/smtp/accesses",
        Some(serde_json::json!({ "name": "Staging" })),
    )
    .await;
    let identifier = created["access"]["identifier"].as_str().unwrap().to_owned();
    for (email, role) in [
        ("viewer@example.test", GlobalRole::Viewer),
        ("member@example.test", GlobalRole::Member),
    ] {
        let application = member_router(&state, &owner_token, email, role).await;
        for (method, uri, body) in [
            ("GET", "/api/v1/smtp/accesses".to_owned(), None),
            (
                "POST",
                "/api/v1/smtp/accesses".to_owned(),
                Some(serde_json::json!({ "name": "Sneaky" })),
            ),
            (
                "POST",
                format!("/api/v1/smtp/accesses/{identifier}/password"),
                None,
            ),
            (
                "PUT",
                format!("/api/v1/smtp/accesses/{identifier}"),
                Some(serde_json::json!({ "enabled": false })),
            ),
            (
                "DELETE",
                format!("/api/v1/smtp/accesses/{identifier}"),
                None,
            ),
            ("GET", "/api/v1/smtp/server".to_owned(), None),
        ] {
            let (status, _, body) = exchange(&application, method, &uri, body).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{role:?} {method} {uri}");
            assert!(body.get("password").is_none());
        }
    }
    let (_, _, listed) = exchange(&owner, "GET", "/api/v1/smtp/accesses", None).await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["enabled"], true);
}

/// Unauthenticated callers cannot reach SMTP administration at all.
#[tokio::test]
async fn smtp_administration_requires_a_session() {
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, _) = initialized_state(memory_storage().await, events).await;
    let application = router(state);
    let (status, _, _) = exchange(&application, "GET", "/api/v1/smtp/accesses", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// Access rules travel as `requires_encryption` and lowercase `allowed_mechanisms`, update
/// partially, and an empty mechanism set is rejected.
#[tokio::test]
async fn smtp_access_rules_round_trip_through_http() {
    let application = initialized_router(memory_storage().await).await;
    let (status, _, created) = exchange(
        &application,
        "POST",
        "/api/v1/smtp/accesses",
        Some(serde_json::json!({
            "name": "Strict",
            "requires_encryption": true,
            "allowed_mechanisms": ["login"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["access"]["requires_encryption"], true);
    assert_eq!(
        created["access"]["allowed_mechanisms"],
        serde_json::json!(["login"])
    );
    let identifier = created["access"]["identifier"].as_str().unwrap().to_owned();
    let uri = format!("/api/v1/smtp/accesses/{identifier}");

    let (status, _, updated) = exchange(
        &application,
        "PUT",
        &uri,
        Some(serde_json::json!({ "allowed_mechanisms": ["plain", "login"] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        updated["requires_encryption"], true,
        "omitted fields are kept"
    );
    assert_eq!(
        updated["allowed_mechanisms"],
        serde_json::json!(["plain", "login"])
    );
    assert_eq!(updated["enabled"], true);

    let (status, _, _) = exchange(
        &application,
        "PUT",
        &uri,
        Some(serde_json::json!({ "allowed_mechanisms": [] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = exchange(
        &application,
        "PUT",
        &uri,
        Some(serde_json::json!({ "allowed_mechanisms": ["cram-md5"] })),
    )
    .await;
    assert!(
        status.is_client_error(),
        "unknown mechanisms are rejected: {status}"
    );

    let (_, _, defaults) = exchange(
        &application,
        "POST",
        "/api/v1/smtp/accesses",
        Some(serde_json::json!({ "name": "Defaults" })),
    )
    .await;
    assert_eq!(defaults["access"]["requires_encryption"], false);
    assert_eq!(
        defaults["access"]["allowed_mechanisms"],
        serde_json::json!(["plain", "login"])
    );
}

/// The SMTP listener is readable but has no HTTP mutation operation.
#[tokio::test]
async fn smtp_server_configuration_is_read_only() {
    let application = initialized_router(memory_storage().await).await;
    let (status, _, before) = exchange(&application, "GET", "/api/v1/smtp/server", None).await;
    assert_eq!(status, StatusCode::OK);
    for address in [serde_json::json!("0.0.0.0:2525"), serde_json::Value::Null] {
        let (status, _, _) = exchange(
            &application,
            "PUT",
            "/api/v1/smtp/server",
            Some(serde_json::json!({ "listen_address": address })),
        )
        .await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    }
    let (status, _, after) = exchange(&application, "GET", "/api/v1/smtp/server", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after, before);
}
