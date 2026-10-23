//! HTTP administration of SMTP Outbounds, routing actions and delivery status.
use super::smtp::{exchange, member_router};
use super::*;
use sandpost_application::{OutboundSecretKey, OutboundSettings};

/**
 * Build an Owner router whose application can seal SMTP Outbound passwords, with its state.
 */
async fn outbound_router() -> (Router, ApplicationState, String) {
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, token) = initialized_state(memory_storage().await, events).await;
    let state = state.with_outbound_settings(OutboundSettings {
        secret_key: Some(OutboundSecretKey::generate().unwrap()),
        ..OutboundSettings::default()
    });
    (
        with_test_auth(router(state.clone()), token.clone()),
        state,
        token,
    )
}

/**
 * Owners create, read, enable, reference and delete SMTP Outbounds without ever reading a
 * password back, and referenced outbounds cannot be deleted.
 */
#[tokio::test]
async fn owners_manage_outbounds_and_routing_actions_over_http() {
    let (application, _, _) = outbound_router().await;
    let body = serde_json::json!({
        "name": "Pippo",
        "host": "smtp.example.com",
        "port": 587,
        "security": "starttls",
        "auth_type": "plain",
        "username": "relay",
        "password": "do-not-echo-this",
        "allowed_recipients": ["*@example.com"],
    });
    let (status, _, created) = exchange(
        &application,
        "POST",
        "/api/v1/smtp/outbounds",
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["enabled"], false, "new outbounds start disabled");
    assert_eq!(created["password_configured"], true);
    assert_eq!(created["tls_verify"], true);
    assert_eq!(created["routing_action_count"], 0);
    let identifier = created["id"].as_str().unwrap().to_owned();
    for uri in [
        "/api/v1/smtp/outbounds".to_owned(),
        format!("/api/v1/smtp/outbounds/{identifier}"),
    ] {
        let (status, _, read) = exchange(&application, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!read.to_string().contains("do-not-echo-this"), "{uri}");
    }
    assert!(!created.to_string().contains("do-not-echo-this"));
    let (status, _, duplicate) =
        exchange(&application, "POST", "/api/v1/smtp/outbounds", Some(body)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{duplicate}");

    let (status, _, enabled) = exchange(
        &application,
        "PUT",
        &format!("/api/v1/smtp/outbounds/{identifier}/enabled"),
        Some(serde_json::json!({ "enabled": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(enabled["enabled"], true);

    let (status, _, scope) = exchange(
        &application,
        "POST",
        "/api/v1/scopes",
        Some(serde_json::json!({
            "id": sandpost_core::ScopeIdentifier::new(),
            "parent": null,
            "name": "Scope A",
            "description": null,
            "filter": "",
            "position": 0,
            "policy_version": 0
        })),
    )
    .await;
    assert!(status.is_success(), "{scope}");
    let scope = scope["id"].as_str().unwrap().to_owned();
    let (status, _, refused) = exchange(
        &application,
        "POST",
        &format!("/api/v1/scopes/{scope}/routing-actions"),
        Some(serde_json::json!({
            "smtp_outbound_id": identifier,
            "action_type": "forward",
            "forward_recipients": ["qa@elsewhere.test"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let (status, _, action) = exchange(
        &application,
        "POST",
        &format!("/api/v1/scopes/{scope}/routing-actions"),
        Some(serde_json::json!({
            "smtp_outbound_id": identifier,
            "action_type": "forward",
            "forward_recipients": ["qa@example.com"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{action}");
    assert_eq!(action["enabled"], true);
    let action = action["id"].as_str().unwrap().to_owned();
    let (status, _, listed) = exchange(
        &application,
        "GET",
        &format!("/api/v1/scopes/{scope}/routing-actions"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let (status, _, in_use) = exchange(
        &application,
        "DELETE",
        &format!("/api/v1/smtp/outbounds/{identifier}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(in_use["error"].as_str().unwrap().contains("routing action"));
    let (status, _, _) = exchange(
        &application,
        "DELETE",
        &format!("/api/v1/routing-actions/{action}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, _) = exchange(
        &application,
        "DELETE",
        &format!("/api/v1/smtp/outbounds/{identifier}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _, policy) = exchange(&application, "GET", "/api/v1/outbound/policy", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(policy["allow_internal_addresses"], false);
    assert_eq!(policy["credentials_available"], true);
    let (status, _, deliveries) = exchange(
        &application,
        "GET",
        "/api/v1/outbound/deliveries?limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(deliveries, serde_json::json!([]));
}

/**
 * Members, Viewers and anonymous callers cannot use any outbound route, and draft connection
 * tests to internal addresses are refused by the default destination policy.
 */
#[tokio::test]
async fn outbound_routes_require_an_owner() {
    let (application, state, token) = outbound_router().await;
    let routes = [
        ("GET", "/api/v1/outbound/policy"),
        ("GET", "/api/v1/smtp/outbounds"),
        ("POST", "/api/v1/smtp/outbounds"),
        ("POST", "/api/v1/smtp/outbounds/test"),
        ("GET", "/api/v1/outbound/deliveries"),
    ];
    let draft = serde_json::json!({
        "name": "Draft",
        "host": "127.0.0.1",
        "port": 25,
        "security": "none",
        "auth_type": "none"
    });
    for (role, email) in [
        (GlobalRole::Member, "member@example.test"),
        (GlobalRole::Viewer, "viewer@example.test"),
    ] {
        let restricted = member_router(&state, &token, email, role).await;
        for (method, uri) in routes {
            let body = (method == "POST").then(|| draft.clone());
            let (status, _, _) = exchange(&restricted, method, uri, body).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{role:?} {method} {uri}");
        }
    }
    let anonymous = router(state.clone());
    for (method, uri) in routes {
        let body = (method == "POST").then(|| draft.clone());
        let (status, _, _) = exchange(&anonymous, method, uri, body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
    }
    let (status, _, tested) = exchange(
        &application,
        "POST",
        "/api/v1/smtp/outbounds/test",
        Some(draft),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tested["success"], false);
    assert_eq!(tested["error"]["kind"], "policy");
}
