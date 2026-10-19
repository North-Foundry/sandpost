use super::*;

/// Resynchronize another open tab after placement, single deletion, and bulk deletion commits.
#[tokio::test]
async fn committed_mail_changes_resynchronize_open_streams_and_recheck_sessions() {
    let storage = memory_storage().await;
    let identifiers = [MessageIdentifier::new(), MessageIdentifier::new()];
    for identifier in identifiers {
        storage
            .insert_message(&Message {
                identifier,
                facts: MessageFacts::default(),
                raw_message: b"Subject: shared\r\n\r\nbody".to_vec(),
                attachments: vec![],
            })
            .await
            .unwrap();
    }
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, token) = initialized_state(storage, events).await;
    let user = state.instance().session(&token).await.unwrap();
    let folder = state
        .authorized(&user)
        .create_web_folder(sandpost_application::WebFolderRequest {
            name: "Filed".into(),
            icon: None,
            exposed_accounts: vec![],
        })
        .await
        .unwrap();
    let application = with_test_auth(router(state.clone()), token.clone());
    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let ready = body.frame().await.unwrap().unwrap();
    assert!(
        std::str::from_utf8(ready.data_ref().unwrap())
            .unwrap()
            .contains("event: ready")
    );

    for (method, path, payload, expected_status) in [
        (
            "POST",
            "/api/v1/messages/placement".to_owned(),
            serde_json::json!({
                "identifiers": [identifiers[0]],
                "folderIdentifier": folder.identifier,
            }),
            StatusCode::OK,
        ),
        (
            "DELETE",
            format!("/api/v1/messages/{}", identifiers[0]),
            serde_json::Value::Null,
            StatusCode::NO_CONTENT,
        ),
        (
            "DELETE",
            "/api/v1/messages".to_owned(),
            serde_json::Value::Null,
            StatusCode::OK,
        ),
    ] {
        let response = application
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&path)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected_status, "{method} {path}");
        let frame = tokio::time::timeout(Duration::from_secs(1), body.frame())
            .await
            .expect("committed changes should wake the other tab")
            .unwrap()
            .unwrap();
        assert_eq!(
            std::str::from_utf8(frame.data_ref().unwrap()).unwrap(),
            "event: resync\ndata: {}\n\n",
            "refresh hints must not expose message identifiers"
        );
    }

    state.instance().logout(&token).await.unwrap();
    state.notify_change(sandpost_application::ApplicationChange::Visibility);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), body.frame())
            .await
            .unwrap()
            .is_none(),
        "committed changes must revalidate the subscriber's session"
    );
}

/// Keep unauthorized message identifiers out of SSE and stop delivery after logout.
#[tokio::test]
async fn events_filter_invisible_messages_and_recheck_the_session() {
    let storage = memory_storage().await;
    let identifier = MessageIdentifier::new();
    storage
        .insert_message(&Message {
            identifier,
            facts: MessageFacts::default(),
            raw_message: b"Subject: private\r\n\r\nprivate".to_vec(),
            attachments: vec![],
        })
        .await
        .unwrap();
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, owner_token) = initialized_state(storage, events.clone()).await;
    let owner = state.instance().session(&owner_token).await.unwrap();
    state
        .authorized(&owner)
        .create_user(NewAccount {
            name: "Limited User".into(),
            email: "events-limited@example.test".into(),
            password: "limited-user-password".into(),
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
            email: "events-limited@example.test".into(),
            password: "limited-user-password".into(),
        })
        .await
        .unwrap()
        .token;
    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/api/v1/events")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let ready = body.frame().await.unwrap().unwrap();
    assert!(
        std::str::from_utf8(ready.data_ref().unwrap())
            .unwrap()
            .contains("event: ready")
    );

    events
        .send(MessageEvent {
            identifier,
            sequence: MessageSequence(1),
        })
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(60), body.frame())
            .await
            .is_err()
    );

    state.instance().logout(&token).await.unwrap();
    events
        .send(MessageEvent {
            identifier,
            sequence: MessageSequence(2),
        })
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), body.frame())
            .await
            .unwrap()
            .is_none()
    );
}

/// Verify an open SSE stream ends as soon as a revoked session is rechecked.
#[tokio::test]
async fn revoked_session_terminates_an_open_server_sent_event_stream() {
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, token) = initialized_state(memory_storage().await, events.clone()).await;
    let application = router(state.clone());
    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/events")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let ready = body.frame().await.unwrap().unwrap();
    assert!(
        std::str::from_utf8(ready.data_ref().unwrap())
            .unwrap()
            .contains("event: ready")
    );

    let response = application
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/logout")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    events
        .send(MessageEvent {
            identifier: MessageIdentifier::new(),
            sequence: MessageSequence(1),
        })
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), body.frame())
            .await
            .unwrap()
            .is_none()
    );
}

/// Verify lagging event subscribers receive a resynchronization event.
#[tokio::test]
async fn slow_server_sent_event_subscribers_receive_resynchronization_instead_of_silent_loss() {
    let (events, _) = tokio::sync::broadcast::channel(1);
    let (state, token) = initialized_state(memory_storage().await, events.clone()).await;
    let application = with_test_auth(router(state.clone()), token);
    let response = application
        .oneshot(
            Request::builder()
                .uri("/api/v1/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let mut body = response.into_body();
    body.frame().await.unwrap().unwrap();
    for sequence in 1..=3 {
        state
            .publish_message(MessageEvent {
                identifier: MessageIdentifier::new(),
                sequence: MessageSequence(sequence),
            })
            .unwrap();
    }
    let frame = tokio::time::timeout(Duration::from_secs(1), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        std::str::from_utf8(frame.data_ref().unwrap())
            .unwrap()
            .contains("event: resync")
    );
}

/// Verify absent frontend routes and the initial server-sent event handshake.
#[tokio::test]
async fn absent_frontend_routes_and_initial_server_sent_event_handshake() {
    let application = build_test_application().await;
    for resource_path in ["/", "/app.js", "/style.css"] {
        let response = application
            .clone()
            .oneshot(
                Request::builder()
                    .uri(resource_path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    let response = application
        .oneshot(
            Request::builder()
                .uri("/api/v1/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    let mut body = response.into_body();
    let frame = tokio::time::timeout(Duration::from_secs(1), body.frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        std::str::from_utf8(frame.data_ref().unwrap())
            .unwrap()
            .contains("event: ready")
    );
}
