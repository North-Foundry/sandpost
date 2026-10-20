//! Persona HTTP contracts: ownership, validation errors, privacy, the mail view, and history.
use super::*;
use sandpost_core::Mailbox;
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
 * Create a user through the Owner API and sign it in, returning its token.
 */
async fn account(application: &Router, owner_token: &str, email: &str, role: &str) -> String {
    let (status, created) = request(
        application,
        "POST",
        "/api/v1/users",
        owner_token,
        Some(json!({
            "name": "Member",
            "email": email,
            "password": "a-member-test-password",
            "role": role,
            "mail_access": "all",
        })),
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
    login["token"].as_str().unwrap().to_owned()
}

/**
 * Build a router over fresh storage, returning it with the storage and the Owner token.
 */
async fn persona_router() -> (Router, Arc<dyn Storage>, String) {
    let storage = memory_storage().await;
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, token) = initialized_state(storage.clone(), events).await;
    (router(state), storage, token)
}

/**
 * Owners of a Persona create, read, update, toggle, validate, and delete it over HTTP.
 */
#[tokio::test]
async fn persona_routes_manage_owned_personas() {
    let (application, _storage, token) = persona_router().await;
    let body = json!({
        "name": "Alice",
        "address": " Alice@Example.test ",
        "script": "wait 5s;\nopen;",
    });
    let (status, created) =
        request(&application, "POST", "/api/v1/personas", &token, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["address"], "alice@example.test");
    assert_eq!(created["enabled"], true);
    assert_eq!(created["revision"], 1);
    assert_eq!(created["address_access"], "confirmed");
    assert_eq!(created["last_execution"], Value::Null);
    let identifier = created["id"].as_str().unwrap().to_owned();
    let path = format!("/api/v1/personas/{identifier}");

    let (status, listed) = request(&application, "GET", "/api/v1/personas", &token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let (status, read) = request(&application, "GET", &path, &token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["script"], "wait 5s;\nopen;");

    let (status, updated) = request(
        &application,
        "PUT",
        &path,
        &token,
        Some(json!({ "name": "Alice 2", "address": "alice@example.test", "script": "open;", "enabled": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(
        (updated["name"].as_str(), updated["revision"].as_u64()),
        (Some("Alice 2"), Some(2))
    );
    let (status, disabled) = request(
        &application,
        "PUT",
        &format!("{path}/enabled"),
        &token,
        Some(json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(disabled["enabled"], false);

    let (status, validation) = request(
        &application,
        "POST",
        "/api/v1/personas/validate",
        &token,
        Some(json!({ "script": "open;\nclick link(foo: \"x\");" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(validation["valid"], false);
    assert_eq!(validation["error"]["line"], 2);
    assert_eq!(validation["error"]["column"], 12);
    assert!(
        validation["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown link argument")
    );
    let (_, valid) = request(
        &application,
        "POST",
        "/api/v1/personas/validate",
        &token,
        Some(json!({ "script": "wait 2s; open;" })),
    )
    .await;
    assert_eq!(
        (
            valid["valid"].as_bool(),
            valid["longest_wait_milliseconds"].as_u64()
        ),
        (Some(true), Some(2000))
    );

    let (status, _) = request(&application, "DELETE", &path, &token, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = request(&application, "GET", &path, &token, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/**
 * Invalid input and duplicate addresses produce actionable errors.
 */
#[tokio::test]
async fn persona_routes_report_actionable_errors() {
    let (application, _storage, token) = persona_router().await;
    let create = |address: &str, script: &str| json!({ "name": "Persona", "address": address, "script": script });
    let (status, error) = request(
        &application,
        "POST",
        "/api/v1/personas",
        &token,
        Some(create("nope", "open;")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("valid ASCII email address")
    );
    let (status, error) = request(
        &application,
        "POST",
        "/api/v1/personas",
        &token,
        Some(create("a@b.test", "open;\nwait 5;")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let message = error["error"].as_str().unwrap();
    assert!(
        message.contains("line 2, column 6") && message.contains("needs a unit"),
        "{message}"
    );
    let (status, _) = request(
        &application,
        "POST",
        "/api/v1/personas",
        &token,
        Some(create("a@b.test", "open;")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, error) = request(
        &application,
        "POST",
        "/api/v1/personas",
        &token,
        Some(create("A@B.test", "open;")),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("already used by a persona")
    );
    let (status, _) = request(
        &application,
        "POST",
        "/api/v1/personas",
        &token,
        Some(json!({ "name": "x", "address": "c@b.test", "script": "open;", "owner_id": "x" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "unknown fields are refused"
    );
    let (status, _) = request(
        &application,
        "GET",
        "/api/v1/personas?owner=me",
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = request(
        &application,
        "GET",
        "/api/v1/personas/not-a-uuid",
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/**
 * Other users cannot reach a Persona by guessing its identifier; Owners administer it.
 */
#[tokio::test]
async fn persona_routes_hide_other_users_personas() {
    let (application, _storage, owner) = persona_router().await;
    let alice = account(&application, &owner, "alice@example.test", "member").await;
    let bob = account(&application, &owner, "bob@example.test", "member").await;
    let viewer = account(&application, &owner, "viewer@example.test", "viewer").await;
    let (_, created) = request(
        &application,
        "POST",
        "/api/v1/personas",
        &alice,
        Some(json!({ "name": "Alice", "address": "persona@example.test", "script": "open;" })),
    )
    .await;
    let identifier = created["id"].as_str().unwrap();
    let path = format!("/api/v1/personas/{identifier}");
    for (method, suffix, body) in [
        ("GET", "", None),
        (
            "PUT",
            "",
            Some(json!({ "name": "x", "address": "x@example.test", "script": "open;" })),
        ),
        ("PUT", "/enabled", Some(json!({ "enabled": false }))),
        ("DELETE", "", None),
        ("GET", "/messages", None),
        ("GET", "/executions", None),
        ("GET", &format!("/executions/{}", uuid_like()), None),
    ] {
        let (status, _) =
            request(&application, method, &format!("{path}{suffix}"), &bob, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {suffix}");
    }
    let (_, bob_list) = request(&application, "GET", "/api/v1/personas", &bob, None).await;
    assert_eq!(bob_list, json!([]));
    let (status, _) = request(
        &application,
        "GET",
        "/api/v1/personas?owner=all",
        &bob,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = request(
        &application,
        "POST",
        "/api/v1/personas",
        &viewer,
        Some(json!({ "name": "V", "address": "v@example.test", "script": "open;" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, all) = request(
        &application,
        "GET",
        "/api/v1/personas?owner=all",
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(all.as_array().unwrap().len(), 1);
    let (status, _) = request(
        &application,
        "PUT",
        &path,
        &owner,
        Some(json!({ "name": "x", "address": "persona@example.test", "script": "wait 1s;" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "Owners cannot rewrite another user's script"
    );
    let (status, _) = request(&application, "DELETE", &path, &owner, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/**
 * Return a random identifier string for a nonexistent execution.
 */
fn uuid_like() -> String {
    MessageIdentifier::new().to_string()
}

/**
 * The Persona view and history expose readable mail addressed to the Persona.
 */
#[tokio::test]
async fn persona_routes_expose_messages_and_history() {
    let (application, storage, token) = persona_router().await;
    let (_, created) = request(
        &application,
        "POST",
        "/api/v1/personas",
        &token,
        Some(json!({ "name": "Alice", "address": "alice@example.test", "script": "open;" })),
    )
    .await;
    let identifier = created["id"].as_str().unwrap().to_owned();
    for (subject, recipient) in [
        ("Welcome", "Alice@example.test"),
        ("Invoice", "alice@example.test"),
        ("Elsewhere", "bob@example.test"),
    ] {
        storage
            .insert_message(&Message {
                identifier: MessageIdentifier::new(),
                facts: MessageFacts {
                    subject: subject.into(),
                    envelope_to: vec![Mailbox {
                        address: recipient.into(),
                        domain: "example.test".into(),
                    }],
                    ..MessageFacts::default()
                },
                raw_message: format!("Subject: {subject}\r\n\r\nbody").into_bytes(),
                attachments: vec![],
            })
            .await
            .unwrap();
    }
    let (status, messages) = request(
        &application,
        "GET",
        &format!("/api/v1/personas/{identifier}/messages?limit=10"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let subjects: Vec<_> = messages
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["subject"].as_str().unwrap())
        .collect();
    assert_eq!(subjects, ["Invoice", "Welcome"]);
    let (_, filtered) = request(
        &application,
        "GET",
        &format!("/api/v1/personas/{identifier}/messages?q=subject%20contains%20%22Wel%22"),
        &token,
        None,
    )
    .await;
    assert_eq!(filtered.as_array().unwrap().len(), 1);

    let (status, history) = request(
        &application,
        "GET",
        &format!("/api/v1/personas/{identifier}/executions?limit=10"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{history}");
    let history = history.as_array().unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0]["status"], "pending");
    assert_eq!(history[0]["subject"], "Invoice");
    let execution = history[0]["id"].as_str().unwrap();
    let (status, details) = request(
        &application,
        "GET",
        &format!("/api/v1/personas/{identifier}/executions/{execution}"),
        &token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(details["actions"], json!([]));
    assert_eq!(details["persona_id"], identifier);
    let (_, persona) = request(
        &application,
        "GET",
        &format!("/api/v1/personas/{identifier}"),
        &token,
        None,
    )
    .await;
    assert_eq!(persona["last_execution"]["subject"], "Invoice");
}

/**
 * Every Persona route requires a session.
 */
#[tokio::test]
async fn persona_routes_require_authentication() {
    let (application, _storage, _token) = persona_router().await;
    let identifier = MessageIdentifier::new();
    for (method, path) in [
        ("GET", "/api/v1/personas".to_owned()),
        ("POST", "/api/v1/personas".to_owned()),
        ("POST", "/api/v1/personas/validate".to_owned()),
        ("GET", format!("/api/v1/personas/{identifier}")),
        ("PUT", format!("/api/v1/personas/{identifier}/enabled")),
        ("GET", format!("/api/v1/personas/{identifier}/messages")),
        ("GET", format!("/api/v1/personas/{identifier}/executions")),
        (
            "GET",
            format!("/api/v1/personas/{identifier}/executions/{identifier}"),
        ),
    ] {
        let (status, body) = request(&application, method, &path, "", Some(json!({}))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
        assert_eq!(body["error"], "authentication_required");
    }
}
