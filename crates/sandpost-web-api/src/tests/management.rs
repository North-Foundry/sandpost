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

/**
 * Reject Member and Viewer access across every instance administration route.
 */
#[tokio::test]
async fn member_and_viewer_cannot_use_administration_routes() {
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, owner_token) = initialized_state(memory_storage().await, events).await;
    let owner = state.instance().session(&owner_token).await.unwrap();
    let authorized = state.authorized(&owner);
    let smtp = authorized
        .create_smtp_access(sandpost_application::NewSmtpAccessRequest {
            name: "Existing".into(),
            username: None,
            requires_encryption: false,
            allowed_mechanisms: sandpost_core::SmtpAuthenticationMechanism::ALL.to_vec(),
        })
        .await
        .unwrap();
    let smtp_id = smtp.access.identifier;
    let scope_id = sandpost_core::ScopeIdentifier::new();
    let view_id = ViewIdentifier::new();
    authorized
        .save_view(View {
            identifier: view_id,
            owner_identifier: None,
            name: "Existing shared view".into(),
            icon: None,
            filter: "true".into(),
        })
        .await
        .unwrap();
    let user_id = authorized
        .create_user(NewAccount {
            name: "Member".into(),
            email: "member-admin-check@example.test".into(),
            password: "member-user-password".into(),
            global_role: GlobalRole::Member,
            mail_access: sandpost_core::MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap()
        .identifier;
    let _viewer_id = authorized
        .create_user(NewAccount {
            name: "Viewer".into(),
            email: "viewer-admin-check@example.test".into(),
            password: "viewer-user-password".into(),
            global_role: GlobalRole::Viewer,
            mail_access: sandpost_core::MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap()
        .identifier;
    let routes = [
        ("GET", "/api/v1/search/status".to_owned(), None),
        ("POST", "/api/v1/search/rebuild".to_owned(), None),
        ("GET", "/api/v1/smtp/server".to_owned(), None),
        ("GET", "/api/v1/smtp/accesses".to_owned(), None),
        (
            "POST",
            "/api/v1/smtp/accesses".to_owned(),
            Some(serde_json::json!({"name":"new"})),
        ),
        (
            "PUT",
            format!("/api/v1/smtp/accesses/{smtp_id}"),
            Some(serde_json::json!({"enabled":false})),
        ),
        ("DELETE", format!("/api/v1/smtp/accesses/{smtp_id}"), None),
        (
            "POST",
            format!("/api/v1/smtp/accesses/{smtp_id}/password"),
            None,
        ),
        (
            "POST",
            "/api/v1/scopes".to_owned(),
            Some(
                serde_json::json!({"id":scope_id,"parent":null,"name":"scope","description":null,"filter":"true","position":0,"policy_version":0}),
            ),
        ),
        (
            "POST",
            "/api/v1/scopes/preview".to_owned(),
            Some(serde_json::json!({"filter":"true"})),
        ),
        (
            "PUT",
            format!("/api/v1/scopes/{scope_id}"),
            Some(
                serde_json::json!({"id":scope_id,"parent":null,"name":"scope","description":null,"filter":"true","position":0,"policy_version":0}),
            ),
        ),
        ("DELETE", format!("/api/v1/scopes/{scope_id}"), None),
        (
            "GET",
            format!("/api/v1/scopes/{scope_id}/assignments"),
            None,
        ),
        (
            "PUT",
            format!("/api/v1/scopes/{scope_id}/assignments/{user_id}"),
            None,
        ),
        (
            "DELETE",
            format!("/api/v1/scopes/{scope_id}/assignments/{user_id}"),
            None,
        ),
        ("GET", "/api/v1/users".to_owned(), None),
        (
            "POST",
            "/api/v1/users".to_owned(),
            Some(
                serde_json::json!({"name":"new","email":"new@example.test","password":"new-user-password","role":"member"}),
            ),
        ),
        (
            "PUT",
            format!("/api/v1/users/{user_id}"),
            Some(serde_json::json!({"name":"changed"})),
        ),
        ("DELETE", format!("/api/v1/users/{user_id}"), None),
        (
            "POST",
            "/api/v1/views".to_owned(),
            Some(
                serde_json::json!({"identifier":view_id,"owner_identifier":null,"name":"shared","filter":"true"}),
            ),
        ),
        (
            "PUT",
            format!("/api/v1/views/{view_id}"),
            Some(
                serde_json::json!({"identifier":view_id,"owner_identifier":null,"name":"shared","filter":"true"}),
            ),
        ),
        ("DELETE", format!("/api/v1/views/{view_id}"), None),
    ];
    for (email, password, role) in [
        (
            "member-admin-check@example.test",
            "member-user-password",
            GlobalRole::Member,
        ),
        (
            "viewer-admin-check@example.test",
            "viewer-user-password",
            GlobalRole::Viewer,
        ),
    ] {
        let token = state
            .instance()
            .login(LoginRequest {
                remember_me: false,
                email: email.into(),
                password: password.into(),
            })
            .await
            .unwrap()
            .token;
        let api = with_test_auth(router(state.clone()), token);
        for (method, uri, body) in &routes {
            let (status, _) = json_request(&api, method, uri, body.clone()).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{role:?} {method} {uri}");
        }
    }
}

/**
 * The removed Admin role is rejected at the HTTP JSON boundary for account creation and update.
 */
#[tokio::test]
async fn admin_role_payload_is_rejected() {
    let application = initialized_router(memory_storage().await).await;
    for (method, uri, body) in [
        (
            "POST",
            "/api/v1/users".to_owned(),
            serde_json::json!({"name":"Old Admin","email":"old-admin@example.test","password":"old-admin-password","role":"admin"}),
        ),
        (
            "PUT",
            "/api/v1/users/00000000-0000-0000-0000-000000000001".to_owned(),
            serde_json::json!({"role":"admin"}),
        ),
    ] {
        let (status, _) = json_request(&application, method, &uri, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{method} {uri}");
    }
}

/**
 * Preserve an owner through promotion and demotion, and reject removing the final owner.
 */
#[tokio::test]
async fn owner_promotions_demotions_and_last_owner_are_enforced_over_http() {
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, owner_token) = initialized_state(memory_storage().await, events).await;
    let owner = state.instance().session(&owner_token).await.unwrap();
    let created = state
        .authorized(&owner)
        .create_user(NewAccount {
            name: "Second Owner".into(),
            email: "second-owner@example.test".into(),
            password: "second-owner-password".into(),
            global_role: GlobalRole::Member,
            mail_access: sandpost_core::MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    let first_api = with_test_auth(router(state.clone()), owner_token);
    let (status, _) = json_request(
        &first_api,
        "PUT",
        &format!("/api/v1/users/{}", created.identifier),
        Some(serde_json::json!({"role":"owner"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let second_token = state
        .instance()
        .login(LoginRequest {
            remember_me: false,
            email: "second-owner@example.test".into(),
            password: "second-owner-password".into(),
        })
        .await
        .unwrap()
        .token;
    let second_api = with_test_auth(router(state.clone()), second_token);
    let (status, _) = json_request(
        &second_api,
        "PUT",
        &format!("/api/v1/users/{}", owner.id),
        Some(serde_json::json!({"role":"member"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "another owner can demote the first owner"
    );
    let (status, _) = json_request(
        &second_api,
        "PUT",
        &format!("/api/v1/users/{}", created.identifier),
        Some(serde_json::json!({"role":"member"})),
    )
    .await;
    assert!(!status.is_success(), "the final owner cannot be demoted");
    let (status, _) = json_request(
        &second_api,
        "DELETE",
        &format!("/api/v1/users/{}", created.identifier),
        None,
    )
    .await;
    assert!(!status.is_success(), "the final owner cannot be deleted");
}

/**
 * Block Viewer mutations to flags, folders, Views, and owned IMAP configuration without
 * treating read requests as implicit Seen updates.
 */
#[tokio::test]
async fn viewer_mutations_are_denied_and_get_does_not_mark_seen() {
    let storage = memory_storage().await;
    let message_id = sandpost_core::MessageIdentifier::new();
    storage
        .insert_message(&sandpost_core::Message {
            identifier: message_id,
            facts: sandpost_core::MessageFacts {
                subject: "Still unread".into(),
                ..Default::default()
            },
            raw_message: b"Subject: Still unread\r\n\r\nbody".to_vec(),
            attachments: vec![],
        })
        .await
        .unwrap();
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, owner_token) = initialized_state(storage, events).await;
    let owner = state.instance().session(&owner_token).await.unwrap();
    let account = state
        .authorized(&owner)
        .create_user(NewAccount {
            name: "Future Viewer".into(),
            email: "future-viewer@example.test".into(),
            password: "future-viewer-password".into(),
            global_role: GlobalRole::Member,
            mail_access: sandpost_core::MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    let member_token = state
        .instance()
        .login(LoginRequest {
            remember_me: false,
            email: "future-viewer@example.test".into(),
            password: "future-viewer-password".into(),
        })
        .await
        .unwrap()
        .token;
    let member_api = with_test_auth(router(state.clone()), member_token);
    let (status, imap_account) = json_request(
        &member_api,
        "POST",
        "/api/v1/imap/accounts",
        Some(serde_json::json!({"name":"Personal Mail"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let imap_id = imap_account["account"]["identifier"]
        .as_str()
        .unwrap()
        .to_owned();
    let owner_api = with_test_auth(router(state.clone()), owner_token);
    let (status, _) = json_request(
        &owner_api,
        "PUT",
        &format!("/api/v1/users/{}", account.identifier),
        Some(serde_json::json!({"role":"viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let viewer_token = state
        .instance()
        .login(LoginRequest {
            remember_me: false,
            email: "future-viewer@example.test".into(),
            password: "future-viewer-password".into(),
        })
        .await
        .unwrap()
        .token;
    let viewer_api = with_test_auth(router(state), viewer_token);

    let (status, rows) = json_request(&viewer_api, "GET", "/api/v1/messages", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rows[0]["seen"], false);
    let (status, _) = json_request(
        &viewer_api,
        "GET",
        &format!("/api/v1/messages/{message_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, rows) = json_request(&viewer_api, "GET", "/api/v1/messages", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rows[0]["seen"], false, "GET reads must not mutate Seen");

    let folder_id = sandpost_core::ImapFolderIdentifier::new();
    let view_id = ViewIdentifier::new();
    for (method, uri, body) in [
        (
            "POST",
            "/api/v1/messages/seen".to_owned(),
            Some(serde_json::json!({"identifiers":[message_id],"seen":true})),
        ),
        (
            "POST",
            "/api/v1/messages/flags".to_owned(),
            Some(
                serde_json::json!({"identifiers":[message_id],"operation":"add","flags":{"flagged":true}}),
            ),
        ),
        (
            "POST",
            "/api/v1/folders".to_owned(),
            Some(serde_json::json!({"name":"Denied","exposedAccounts":[]})),
        ),
        (
            "PUT",
            format!("/api/v1/folders/{folder_id}"),
            Some(serde_json::json!({"name":"Denied","exposedAccounts":[]})),
        ),
        ("DELETE", format!("/api/v1/folders/{folder_id}"), None),
        (
            "POST",
            "/api/v1/views".to_owned(),
            Some(
                serde_json::json!({"identifier":view_id,"owner_identifier":account.identifier,"name":"Personal","filter":"true"}),
            ),
        ),
        (
            "PUT",
            format!("/api/v1/views/{view_id}"),
            Some(
                serde_json::json!({"identifier":view_id,"owner_identifier":account.identifier,"name":"Personal","filter":"true"}),
            ),
        ),
        ("DELETE", format!("/api/v1/views/{view_id}"), None),
        (
            "PUT",
            format!("/api/v1/imap/accounts/{imap_id}"),
            Some(serde_json::json!({"enabled":false,"name":"Changed"})),
        ),
        (
            "POST",
            "/api/v1/imap/accounts".to_owned(),
            Some(serde_json::json!({"name":"Extra"})),
        ),
    ] {
        let (status, _) = json_request(&viewer_api, method, &uri, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
}
