use super::*;

/// Deny account management and single or bulk message deletion to an authenticated viewer.
#[tokio::test]
async fn viewer_cannot_manage_accounts_or_delete_messages() {
    let storage = memory_storage().await;
    let message_identifier = MessageIdentifier::new();
    storage
        .insert_message(&Message {
            identifier: message_identifier,
            facts: MessageFacts::default(),
            raw_message: b"Subject: limited\r\n\r\nbody".to_vec(),
            attachments: vec![],
        })
        .await
        .unwrap();
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, owner_token) = initialized_state(storage, events).await;
    let owner = state.instance().session(&owner_token).await.unwrap();
    state
        .authorized(&owner)
        .create_user(NewAccount {
            name: "Limited User".into(),
            email: "limited@example.test".into(),
            password: "limited-user-password".into(),
            global_role: GlobalRole::Viewer,
            mail_access: sandpost_core::MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    let token = state
        .instance()
        .login(LoginRequest {
            remember_me: false,
            email: "limited@example.test".into(),
            password: "limited-user-password".into(),
        })
        .await
        .unwrap()
        .token;
    let application = with_test_auth(router(state), token);

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/users")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/search/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    for uri in [
        format!("/api/v1/messages/{message_identifier}"),
        "/api/v1/messages".to_owned(),
    ] {
        let response = application
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(&uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri}");
    }
}

/// Send one authenticated JSON request and decode the JSON response body, if any.
async fn json_request(
    application: &Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder().method(method).uri(uri);
    if body.is_some() {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    let response = application
        .clone()
        .oneshot(
            request
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

/// Expose query validation, scope management, and previews; endpoint routes no longer exist.
#[tokio::test]
async fn admin_routes_validate_queries_and_manage_scopes() {
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, owner_token) = initialized_state(memory_storage().await, events).await;
    let anonymous = router(state.clone());
    for (method, uri) in [
        ("POST", "/api/v1/query/validate"),
        ("POST", "/api/v1/scopes/preview"),
    ] {
        let (status, _) = json_request(&anonymous, method, uri, Some(serde_json::json!({}))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}");
    }
    let api = with_test_auth(router(state), owner_token);
    for uri in [
        "/api/v1/endpoints",
        "/api/v1/admin/endpoints",
        "/api/v1/endpoints/00000000-0000-0000-0000-000000000002/users",
    ] {
        let (status, _) = json_request(&api, "GET", uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }

    let (status, validation) = json_request(
        &api,
        "POST",
        "/api/v1/query/validate",
        Some(serde_json::json!({ "query": "subject ==" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(validation["valid"], false);
    assert!(validation["error"]["position"].is_u64());

    let parent = sandpost_core::ScopeIdentifier::new();
    let child = sandpost_core::ScopeIdentifier::new();
    for (identifier, parent_identifier, name) in [
        (parent, serde_json::Value::Null, "EU"),
        (child, serde_json::json!(parent), "Invoices"),
    ] {
        let (status, saved) = json_request(
            &api,
            "POST",
            "/api/v1/scopes",
            Some(serde_json::json!({
                "id": identifier,
                "parent": parent_identifier,
                "name": name,
                "description": null,
                "filter": "to.domain == \"eu.test\"",
                "position": 0,
                "policy_version": 0,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(saved.get("endpoint_identifier").is_none());
    }
    let (status, preview) = json_request(
        &api,
        "POST",
        "/api/v1/scopes/preview",
        Some(serde_json::json!({
            "parent": child,
            "filter": "subject contains \"invoice\"",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(preview.as_array().unwrap().is_empty());

    let (status, in_use) =
        json_request(&api, "DELETE", &format!("/api/v1/scopes/{parent}"), None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(in_use["error"].as_str().unwrap().contains("child scopes"));
}
