use super::*;

/// Public key verification creates no owner, exposes only an authorization, and gates setup.
#[tokio::test]
async fn setup_authorization_endpoint_verifies_keys_without_creating_an_owner() {
    let storage = memory_storage().await;
    let directory = std::env::temp_dir().join(format!(
        "sandpost-web-api-setup-authorization-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let instance = sandpost_application::Instance::open(storage.clone(), directory.clone())
        .await
        .unwrap();
    let secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
    let state = ApplicationState::with_instance(
        storage.clone(),
        tokio::sync::broadcast::channel(16).0,
        instance.clone(),
    )
    .await
    .unwrap();
    let application = router(state);
    let verification_request = |key: &str| {
        Request::builder()
            .method("POST")
            .uri("/api/v1/setup/authorize")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({ "setupKey": key }).to_string(),
            ))
            .unwrap()
    };
    let response = application
        .clone()
        .oneshot(verification_request("wrong-key"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({ "error": "authentication_failed" })
    );
    let response = application
        .clone()
        .oneshot(verification_request(&secret))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert!(!response.headers().contains_key(header::SET_COOKIE));
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let verification: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(verification.as_object().unwrap().len(), 1);
    let authorization = verification["authorization"].as_str().unwrap();
    assert_ne!(authorization, secret);
    assert!(!String::from_utf8_lossy(&body).contains(&secret));
    assert_eq!(storage.count_users().await.unwrap(), 0);
    assert!(!directory.join("instance.json").exists());

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/messages")
                .header(header::AUTHORIZATION, format!("Bearer {authorization}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    for (credential, expected_status) in [
        ("invalid-authorization", StatusCode::UNAUTHORIZED),
        (authorization, StatusCode::CREATED),
        (authorization, StatusCode::CONFLICT),
    ] {
        let response = application
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/setup")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "bootstrapSecret": credential,
                            "name": "Test Owner",
                            "email": TEST_EMAIL,
                            "password": TEST_PASSWORD
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected_status);
    }
    assert_eq!(storage.count_users().await.unwrap(), 1);
    let response = application
        .clone()
        .oneshot(verification_request(&secret))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({ "error": "already_initialized" })
    );
    let response = application
        .oneshot(
            Request::builder()
                .uri("/api/v1/instance")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({ "initialized": true, "setupKeyRequired": false })
    );
    assert!(!String::from_utf8_lossy(&body).contains(&secret));
    assert!(!String::from_utf8_lossy(&body).contains(authorization));
    assert!(instance.bootstrap_secret_for_startup().unwrap().is_none());
    let _ = std::fs::remove_dir_all(directory);
}

/// Verify bootstrap, owner login, cookie and bearer sessions, SSE gating, and logout.
#[tokio::test]
async fn bootstrap_and_session_endpoints_protect_the_http_api() {
    let storage = memory_storage().await;
    let directory = std::env::temp_dir().join(format!(
        "sandpost-web-api-bootstrap-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let instance = sandpost_application::Instance::open(storage.clone(), directory)
        .await
        .unwrap();
    let secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
    let application = router(
        ApplicationState::with_instance(storage, tokio::sync::broadcast::channel(16).0, instance)
            .await
            .unwrap(),
    );

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/instance")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        status,
        serde_json::json!({"initialized": false, "setupKeyRequired": true})
    );
    assert!(
        !body
            .windows(secret.len())
            .any(|window| window == secret.as_bytes())
    );

    for path in [
        "/api/v1/inboxes",
        "/api/v1/messages",
        "/api/v1/search?q=hello",
        "/api/v1/scopes",
        "/api/v1/views",
        "/api/v1/users",
        "/api/v1/search/status",
        "/api/v1/events",
        &format!("/api/v1/messages/{}/raw", MessageIdentifier::new()),
        &format!("/api/v1/messages/{}/html-check", MessageIdentifier::new()),
        &format!("/api/v1/messages/{}/link-check", MessageIdentifier::new()),
        &format!(
            "/api/v1/messages/{}/attachments/0",
            MessageIdentifier::new()
        ),
    ] {
        let response = application
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
    }
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

    let setup_body = |bootstrap_secret: &str| {
        serde_json::json!({
            "bootstrapSecret": bootstrap_secret,
            "name": "Test Owner",
            "email": TEST_EMAIL,
            "password": TEST_PASSWORD
        })
        .to_string()
    };
    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/setup")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(setup_body("wrong-secret")))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"],
        "authentication_failed"
    );

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/setup")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(setup_body(&secret)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/setup")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(setup_body(&secret)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/login")
                .header("x-forwarded-proto", "https")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"email": TEST_EMAIL, "password": TEST_PASSWORD}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let set_cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_owned();
    for attribute in ["HttpOnly", "SameSite=Strict", "Path=/api/v1"] {
        assert!(
            set_cookie.contains(attribute),
            "missing cookie attribute {attribute}"
        );
    }
    assert!(!set_cookie.contains("Max-Age="));
    assert!(!set_cookie.contains("; Secure"));
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let login: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let token = login["token"].as_str().unwrap();
    let user = &login["user"];
    assert_eq!(user["email"], TEST_EMAIL);
    assert_eq!(user["role"], "owner");

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("https://localhost/api/v1/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"email": TEST_EMAIL, "password": TEST_PASSWORD}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("; Secure")
    );

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/session")
                .header(header::COOKIE, &set_cookie[..set_cookie.find(';').unwrap()])
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
                .uri("/api/v1/events")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
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

    let response = application
        .clone()
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
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let response = application
        .oneshot(
            Request::builder()
                .uri("/api/v1/session")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// Verify cookie-authenticated mutations require a matching Origin and no register route exists.
#[tokio::test]
async fn cookie_mutations_check_origin_and_registration_is_absent() {
    let (state, token) = initialized_state(
        memory_storage().await,
        tokio::sync::broadcast::channel(16).0,
    )
    .await;
    let application = router(state);
    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/inboxes")
                .header(header::COOKIE, format!("sandpost_session={token}"))
                .header(header::ORIGIN, "https://attacker.test")
                .header(header::HOST, "localhost")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"name":"Dev","address":"dev@sandpost.local"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/inboxes")
                .header(header::COOKIE, format!("sandpost_session={token}"))
                .header(header::ORIGIN, "http://localhost")
                .header(header::HOST, "localhost")
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
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/register")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/**
 * Remembered cookies restore users after restart, renew on restore, and revoke through logout.
 */
#[tokio::test]
async fn remembered_cookie_restores_and_revokes_a_durable_session() {
    let storage = memory_storage().await;
    let (state, _) =
        initialized_state(storage.clone(), tokio::sync::broadcast::channel(16).0).await;
    let application = router(state);
    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "email": TEST_EMAIL, "password": TEST_PASSWORD, "rememberMe": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(cookie.contains("Max-Age=34560000"));
    let cookie = cookie.split(';').next().unwrap().to_owned();
    drop(application);

    let directory = std::env::temp_dir().join(format!(
        "sandpost-remembered-http-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let instance = sandpost_application::Instance::open(storage.clone(), directory.clone())
        .await
        .unwrap();
    let state =
        ApplicationState::with_instance(storage, tokio::sync::broadcast::channel(16).0, instance)
            .await
            .unwrap();
    let application = router(state);
    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/session")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=34560000")
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["email"],
        TEST_EMAIL
    );

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/logout")
                .header(header::HOST, "localhost")
                .header(header::COOKIE, &cookie)
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
                .method("POST")
                .uri("/api/v1/logout")
                .header(header::HOST, "localhost")
                .header(header::ORIGIN, "http://localhost")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let response = application
        .oneshot(
            Request::builder()
                .uri("/api/v1/session")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let _ = std::fs::remove_dir_all(directory);
}
