//! Link check HTTP contracts: shared reads, message access and bulk starts for every role.
use super::*;
use serde_json::{Value, json};

/**
 * Send a request with a bearer token and decode its JSON response.
 */
async fn request(
    application: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/**
 * Capture one message linking to a loopback address the default policy refuses.
 */
async fn message_with_internal_link(storage: &Arc<dyn Storage>) -> MessageIdentifier {
    let identifier = MessageIdentifier::new();
    storage
        .insert_message(&Message {
            identifier,
            facts: MessageFacts {
                subject: "links".into(),
                markup_body: r#"<a href="http://127.0.0.1:9/reset#token">Reset</a>"#.into(),
                ..MessageFacts::default()
            },
            raw_message: b"Subject: links\r\n\r\nbody".to_vec(),
            attachments: vec![],
        })
        .await
        .unwrap();
    identifier
}

/**
 * Create a user through the Owner API and sign it in, returning its identifier and token.
 */
async fn reader(
    application: &Router,
    owner_token: &str,
    email: &str,
    role: &str,
    mail_access: &str,
) -> (String, String) {
    let account = json!({
        "name": "Member",
        "email": email,
        "password": "a-member-test-password",
        "role": role,
        "mail_access": mail_access,
    });
    let (status, created) = request(
        application,
        "POST",
        "/api/v1/users",
        owner_token,
        Some(account),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let (status, login) = request(
        application,
        "POST",
        "/api/v1/login",
        "",
        Some(json!({ "email": email, "password": "a-member-test-password" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    (
        created["identifier"].as_str().unwrap().to_owned(),
        login["token"].as_str().unwrap().to_owned(),
    )
}

/**
 * Every role can check readable messages; inaccessible messages remain protected.
 */
#[tokio::test]
async fn link_check_routes_allow_all_roles_and_enforce_message_access() {
    let storage = memory_storage().await;
    let message = message_with_internal_link(&storage).await;
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, owner_token) = initialized_state(storage, events).await;
    let application = router(state);
    let path = format!("/api/v1/messages/{message}/link-check");

    let (status, session) =
        request(&application, "GET", "/api/v1/session", &owner_token, None).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    assert_eq!(session["role"], "owner");

    let (status, unchecked) = request(&application, "GET", &path, &owner_token, None).await;
    assert_eq!(status, StatusCode::OK, "{unchecked}");
    assert_eq!(unchecked["checked_at"], Value::Null);
    assert_eq!(unchecked["running"], false);
    assert_eq!(unchecked["summary"]["total"], 0);
    assert_eq!(unchecked["urls"], json!(["http://127.0.0.1:9/reset"]));
    assert_eq!(unchecked["links"], json!([]));

    let (status, started) = request(&application, "POST", &path, &owner_token, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    assert_eq!(started["message_id"], message.to_string());
    let mut report = started;
    for _ in 0..200 {
        if report["running"] == false {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        report = request(&application, "GET", &path, &owner_token, None)
            .await
            .1;
    }
    assert_eq!(report["running"], false);
    assert_eq!(report["summary"]["total"], 1);
    assert_eq!(report["summary"]["inconclusive"], 1);
    let link = &report["links"][0];
    assert_eq!(link["url"], "http://127.0.0.1:9/reset");
    assert_eq!(link["outcome"], "inconclusive");
    assert_eq!(link["error_kind"], "blocked_address");
    assert_eq!(link["status_code"], Value::Null);
    assert!(link["checked_at"].as_i64().unwrap() > 0);

    for role in ["member", "viewer"] {
        let (_, token) = reader(
            &application,
            &owner_token,
            &format!("{role}@example.test"),
            role,
            "all",
        )
        .await;
        let (status, shared) = request(&application, "GET", &path, &token, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(shared["summary"], report["summary"]);
        assert_eq!(shared["links"][0]["url"], report["links"][0]["url"]);
        assert_eq!(shared["links"][0]["outcome"], "inconclusive");
        let (status, _) = request(&application, "POST", &path, &token, None).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let (status, _) = request(
            &application,
            "POST",
            "/api/v1/messages/link-check",
            &token,
            Some(json!({ "identifiers": [message] })),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
    }

    let (_, scoped_token) = reader(
        &application,
        &owner_token,
        "scoped@example.test",
        "viewer",
        "scoped",
    )
    .await;
    for method in ["GET", "POST"] {
        let (status, _) = request(&application, method, &path, &scoped_token, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = request(&application, method, &path, "", None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, skipped) = request(
        &application,
        "POST",
        "/api/v1/messages/link-check",
        &scoped_token,
        Some(json!({ "identifiers": [message] })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        skipped,
        json!({ "started": 0, "already_running": 0, "skipped": 1 })
    );

    let (status, bulk) = request(
        &application,
        "POST",
        "/api/v1/messages/link-check",
        &owner_token,
        Some(json!({ "identifiers": [message, MessageIdentifier::new()] })),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{bulk}");
    assert_eq!(bulk["skipped"], 1);
    assert_eq!(
        bulk["started"].as_u64().unwrap() + bulk["already_running"].as_u64().unwrap(),
        1
    );
    let (status, _) = request(
        &application,
        "POST",
        "/api/v1/messages/link-check",
        &owner_token,
        Some(json!({ "identifiers": [] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = request(
        &application,
        "GET",
        &format!("/api/v1/messages/{}/link-check", MessageIdentifier::new()),
        &owner_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
