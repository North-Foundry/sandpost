use super::*;

/// Fail closed across every message transport for a scoped member without assignments; deletion
/// leaves captured mail physically stored.
#[tokio::test]
async fn unassigned_members_cannot_list_read_or_delete_captured_messages() {
    let storage = memory_storage().await;
    let identifier = MessageIdentifier::new();
    let raw_message = b"Subject: private mail\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=part\r\n\r\n--part\r\nContent-Type: text/plain\r\n\r\nprivate body\r\n--part\r\nContent-Type: text/plain; name=private.txt\r\nContent-Disposition: attachment; filename=private.txt\r\n\r\nprivate attachment\r\n--part--\r\n";
    storage
        .insert_message(&Message {
            identifier,
            facts: MessageFacts {
                subject: "private mail".into(),
                size: raw_message.len() as u64,
                attachment_count: 1,
                ..MessageFacts::default()
            },
            raw_message: raw_message.to_vec(),
            attachments: vec![sandpost_core::Attachment {
                filename: Some("private.txt".into()),
                content_type: "text/plain".into(),
                size: 18,
                content_hash: String::new(),
            }],
        })
        .await
        .unwrap();
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, owner_token) = initialized_state(storage.clone(), events).await;
    let owner = state.instance().session(&owner_token).await.unwrap();
    let owner_application = with_test_auth(router(state.clone()), owner_token);
    for suffix in ["", "/raw", "/attachments/0"] {
        let response = owner_application
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/messages/{identifier}{suffix}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{suffix}");
    }
    state
        .authorized(&owner)
        .create_user(NewAccount {
            name: "Unassigned Member".into(),
            email: "unassigned@example.test".into(),
            password: "unassigned-member-password".into(),
            global_role: GlobalRole::Member,
            mail_access: sandpost_core::MailAccess::Scoped,
            personal_filter: None,
        })
        .await
        .unwrap();
    let token = state
        .instance()
        .login(LoginRequest {
            remember_me: false,
            email: "unassigned@example.test".into(),
            password: "unassigned-member-password".into(),
        })
        .await
        .unwrap()
        .token;
    let member = state.instance().session(&token).await.unwrap();
    let view = state
        .authorized(&member)
        .save_view(View {
            identifier: ViewIdentifier::new(),
            owner_identifier: Some(member.id),
            name: "All mail filter".into(),
            icon: None,
            filter: "true".into(),
        })
        .await
        .unwrap();
    let application = with_test_auth(router(state), token);

    for uri in [
        "/api/v1/messages".to_owned(),
        "/api/v1/messages?q=true".to_owned(),
        format!("/api/v1/messages?view_id={}&q=true", view.identifier),
        format!("/api/v1/messages?inbox_id={}", view.identifier),
        "/api/v1/search?q=true".to_owned(),
        format!("/api/v1/search?q=true&view_id={}", view.identifier),
    ] {
        let response = application
            .clone()
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let messages: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(messages, serde_json::json!([]), "{uri}");
    }
    for (method, suffix) in [
        ("GET", ""),
        ("GET", "/raw"),
        ("GET", "/attachments/0"),
        ("DELETE", ""),
    ] {
        let uri = format!("/api/v1/messages/{identifier}{suffix}");
        let response = application
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method} {uri}");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let error: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(error, serde_json::json!({"error": "item not found"}));
    }
    let response = application
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/messages")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let deleted: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(deleted, serde_json::json!({"deleted": 0}));
    assert!(storage.get_message(identifier).await.unwrap().is_some());

    let response = owner_application
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/messages")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let deleted: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(deleted, serde_json::json!({"deleted": 1}));
    assert!(storage.get_message(identifier).await.unwrap().is_some());
    let response = owner_application
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/messages")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let deleted: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(deleted, serde_json::json!({"deleted": 0}));
}

/// Create and sign in a test account through the same persisted account/session path as HTTP.
async fn create_authenticated_user(
    state: &ApplicationState,
    owner_token: &str,
    name: &str,
    email: &str,
    role: GlobalRole,
    mail_access: sandpost_core::MailAccess,
) -> String {
    let owner = state.instance().session(owner_token).await.unwrap();
    state
        .authorized(&owner)
        .create_user(NewAccount {
            name: name.into(),
            email: email.into(),
            password: "acceptance-test-password".into(),
            global_role: role,
            mail_access,
            personal_filter: None,
        })
        .await
        .unwrap();
    state
        .instance()
        .login(LoginRequest {
            remember_me: false,
            email: email.into(),
            password: "acceptance-test-password".into(),
        })
        .await
        .unwrap()
        .token
}

/// Keep user tombstones isolated across HTTP reads, search, direct retrieval, and bulk deletion.
#[tokio::test]
async fn user_deletion_is_private_and_never_physically_removes_shared_mail() {
    let storage = memory_storage().await;
    let deleted_identifier = MessageIdentifier::new();
    let bulk_identifier = MessageIdentifier::new();
    let raw_message = b"Subject: shared note\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=part\r\n\r\n--part\r\nContent-Type: text/plain\r\n\r\nshared body\r\n--part\r\nContent-Type: text/plain; name=note.txt\r\nContent-Disposition: attachment; filename=note.txt\r\n\r\nshared attachment\r\n--part--\r\n";
    for identifier in [deleted_identifier, bulk_identifier] {
        storage
            .insert_message(&Message {
                identifier,
                facts: MessageFacts {
                    subject: "shared note".into(),
                    size: raw_message.len() as u64,
                    attachment_count: 1,
                    ..MessageFacts::default()
                },
                raw_message: raw_message.to_vec(),
                attachments: vec![sandpost_core::Attachment {
                    filename: Some("note.txt".into()),
                    content_type: "text/plain".into(),
                    size: 18,
                    content_hash: String::new(),
                }],
            })
            .await
            .unwrap();
    }
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, owner_token) = initialized_state(storage.clone(), events).await;
    let user_a_token = create_authenticated_user(
        &state,
        &owner_token,
        "User A",
        "user-a@example.test",
        GlobalRole::Member,
        sandpost_core::MailAccess::All,
    )
    .await;
    let user_b_token = create_authenticated_user(
        &state,
        &owner_token,
        "User B",
        "user-b@example.test",
        GlobalRole::Member,
        sandpost_core::MailAccess::All,
    )
    .await;
    let unauthorized_token = create_authenticated_user(
        &state,
        &owner_token,
        "Unassigned",
        "unassigned-delete@example.test",
        GlobalRole::Member,
        sandpost_core::MailAccess::Scoped,
    )
    .await;
    let user_a = with_test_auth(router(state.clone()), user_a_token);
    let user_b = with_test_auth(router(state.clone()), user_b_token);
    let unauthorized = with_test_auth(router(state), unauthorized_token);

    let response = user_a
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/messages/{deleted_identifier}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        storage
            .get_message(deleted_identifier)
            .await
            .unwrap()
            .is_some()
    );

    let response = user_a
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/messages/{deleted_identifier}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    for uri in [
        "/api/v1/messages".to_owned(),
        "/api/v1/search?q=subject%20contains%20%22shared%20note%22".to_owned(),
    ] {
        let response = user_a
            .clone()
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let rows: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let identifiers: Vec<_> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        assert!(
            !identifiers.contains(&deleted_identifier.to_string().as_str()),
            "{uri}: {rows}"
        );
        assert!(
            identifiers.contains(&bulk_identifier.to_string().as_str()),
            "{uri}: {rows}"
        );
    }
    for suffix in ["", "/raw", "/attachments/0"] {
        let response = user_a
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/messages/{deleted_identifier}{suffix}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{suffix}");
    }

    for uri in [
        "/api/v1/messages".to_owned(),
        "/api/v1/search?q=subject%20contains%20%22shared%20note%22".to_owned(),
    ] {
        let response = user_b
            .clone()
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let rows: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let identifiers: Vec<_> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        for identifier in [deleted_identifier, bulk_identifier] {
            assert!(
                identifiers.contains(&identifier.to_string().as_str()),
                "{uri}: {rows}"
            );
        }
    }
    for suffix in ["", "/raw", "/attachments/0"] {
        let response = user_b
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/messages/{deleted_identifier}{suffix}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{suffix}");
    }

    let response = unauthorized
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/messages/{bulk_identifier}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        storage
            .get_message(deleted_identifier)
            .await
            .unwrap()
            .is_some()
    );

    let response = user_a
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/messages")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let deleted: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(deleted, serde_json::json!({"deleted": 1}));
    assert!(
        storage
            .get_message(bulk_identifier)
            .await
            .unwrap()
            .is_some()
    );

    let response = user_a
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/messages")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let deleted: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(deleted, serde_json::json!({"deleted": 0}));

    for identifier in [deleted_identifier, bulk_identifier] {
        assert!(storage.get_message(identifier).await.unwrap().is_some());
        let response = user_b
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/messages/{identifier}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}

/// Apply the same canonical query globally and through views without leaking views.
#[tokio::test]
async fn search_uses_canonical_dsl_globally_and_with_view_filters() {
    let storage = memory_storage().await;
    let identifier = MessageIdentifier::new();
    storage
        .insert_message(&Message {
            identifier,
            facts: MessageFacts {
                subject: "hello from endpoint".into(),
                ..MessageFacts::default()
            },
            raw_message: b"Subject: hello from endpoint\r\n\r\nbody".to_vec(),
            attachments: vec![],
        })
        .await
        .unwrap();
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, owner_token) = initialized_state(storage, events).await;
    let owner = state.instance().session(&owner_token).await.unwrap();
    let view = state
        .authorized(&owner)
        .save_view(View {
            identifier: ViewIdentifier::new(),
            owner_identifier: Some(owner.id),
            name: "Hello mail".into(),
            icon: None,
            filter: "true".into(),
        })
        .await
        .unwrap();
    let owner_api = with_test_auth(router(state.clone()), owner_token);
    let canonical_query = "content%20contains%20%22hello%22";

    for uri in [
        format!("/api/v1/search?q={canonical_query}"),
        format!(
            "/api/v1/search?q={canonical_query}&view_id={}",
            view.identifier
        ),
    ] {
        let response = owner_api
            .clone()
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let results: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(results[0]["id"], identifier.to_string(), "{uri}");
    }

    for uri in [
        "/api/v1/search?q=hello".to_owned(),
        format!("/api/v1/search?q=hello&view_id={}", view.identifier),
    ] {
        let response = owner_api
            .clone()
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
    }

    state
        .authorized(&owner)
        .create_user(NewAccount {
            name: "View Outsider".into(),
            email: "view-outsider@example.test".into(),
            password: "view-outsider-password".into(),
            global_role: GlobalRole::Member,
            mail_access: sandpost_core::MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    let token = state
        .instance()
        .login(LoginRequest {
            remember_me: false,
            email: "view-outsider@example.test".into(),
            password: "view-outsider-password".into(),
        })
        .await
        .unwrap()
        .token;
    let outsider_api = with_test_auth(router(state), token);
    let response = outsider_api
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v1/search?q={canonical_query}&view_id={}",
                    view.identifier
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// Verify health, listing, detail, raw-message, and pagination error responses.
#[tokio::test]
async fn health_list_detail_raw_and_validation() {
    let application = build_test_application().await;
    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/messages")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let rows: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(rows[0]["subject"], "hello");
    let identifier = rows[0]["id"].as_str().unwrap();
    for suffix in ["", "/raw"] {
        let response = application
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/messages/{identifier}{suffix}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/messages?limit=101")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = application
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/messages/{}", MessageIdentifier::new()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// Verify inbox validation, inbox creation, attachment downloads, and message deletion.
#[tokio::test]
async fn inspector_routes_validate_and_expose_inboxes_attachments_and_deletion() {
    let storage = memory_storage().await;
    let identifier = MessageIdentifier::new();
    let raw_message = b"Subject: hello\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=part\r\n\r\n--part\r\nContent-Type: text/plain\r\n\r\nbody\r\n--part\r\nContent-Type: text/plain; name=note.txt\r\nContent-Disposition: attachment; filename=note.txt\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8gYXR0YWNobWVudA==\r\n--part--\r\n";
    let message = Message {
        identifier,
        facts: MessageFacts {
            subject: "hello".into(),
            size: raw_message.len() as u64,
            attachment_count: 1,
            ..MessageFacts::default()
        },
        raw_message: raw_message.to_vec(),
        attachments: vec![sandpost_core::Attachment {
            filename: Some("note.txt".into()),
            content_type: "text/plain".into(),
            size: 17,
            content_hash: String::new(),
        }],
    };
    storage.insert_message(&message).await.unwrap();
    let application = initialized_router(storage).await;

    for (method, uri, body, expected) in [
        ("GET", "/api/v1/inboxes", Body::empty(), StatusCode::OK),
        (
            "GET",
            "/api/v1/messages?inbox_id=invalid",
            Body::empty(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            "/api/v1/messages?limit=101",
            Body::empty(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            "/api/v1/messages?q=%29",
            Body::empty(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            "/api/v1/messages?q=subject%20contains%20%22hello%22",
            Body::empty(),
            StatusCode::OK,
        ),
        (
            "GET",
            "/api/v1/messages/00000000-0000-0000-0000-000000000001/attachments/-1",
            Body::empty(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            "/api/v1/search?q=content%20contains%20%22hello%22",
            Body::empty(),
            StatusCode::OK,
        ),
        (
            "GET",
            "/api/v1/search?q=hello",
            Body::empty(),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let response = application
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{method} {uri}");
    }

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/search?q=subject%20%3D%3D%20%22hello%22")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let results: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(results[0]["id"], identifier.to_string());

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/search?q=%29")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/inboxes")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"name":"Dev","address":"dev@sandpost.local"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/inboxes")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"name":"Another","address":"dev@sandpost.local"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["error"], "an inbox for this address already exists");

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/messages/{identifier}/attachments/0"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/octet-stream"
    );
    assert_eq!(
        response.headers()[header::CONTENT_DISPOSITION],
        "attachment"
    );
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "hello attachment"
    );

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/messages/{identifier}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = application
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/messages/{identifier}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["error"], "item not found");
}
