use super::*;

/// Verify frontend assets, deep links, and API misses use their proper fallbacks.
#[tokio::test]
async fn frontend_router_serves_assets_and_spa_without_falling_back_for_api() {
    let directory = std::env::temp_dir().join(format!(
        "sandpost-web-api-frontend-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(directory.join("assets")).unwrap();
    std::fs::write(
        directory.join("index.html"),
        "<!doctype html><main>app</main>",
    )
    .unwrap();
    std::fs::write(directory.join("assets/app.js"), "window.app = true;").unwrap();

    let public_frontend = router_with_frontend(
        ApplicationState::new(
            memory_storage().await,
            tokio::sync::broadcast::channel(16).0,
        )
        .await
        .unwrap(),
        directory.clone(),
    );
    let response = public_frontend
        .clone()
        .oneshot(
            Request::builder()
                .uri("/assets/app.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = public_frontend
        .clone()
        .oneshot(
            Request::builder()
                .uri("/deep/link")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = public_frontend
        .oneshot(
            Request::builder()
                .uri("/api/v1/not-a-route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, token) = initialized_state(memory_storage().await, events).await;
    let application = with_test_auth(router_with_frontend(state, directory.clone()), token);
    for (uri, expected_body) in [
        ("/projects/42/messages", "<!doctype html><main>app</main>"),
        ("/assets/app.js", "window.app = true;"),
    ] {
        let response = application
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.into_body().collect().await.unwrap().to_bytes(),
            expected_body
        );
    }

    let response = application
        .oneshot(
            Request::builder()
                .uri("/api/v1/unknown")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

/// Serve the embedded frontend when the build included one, keeping API misses empty.
#[tokio::test]
async fn embedded_frontend_serves_the_entry_point_and_keeps_api_misses_empty() {
    if !crate::EMBEDDED_FRONTEND {
        // The build had no frontend directory, so there is nothing embedded to serve.
        return;
    }
    let (state, token) = initialized_state(
        memory_storage().await,
        tokio::sync::broadcast::channel(16).0,
    )
    .await;
    let application = with_test_auth(router_with_embedded_frontend(state), token);
    let embedded_asset = |name: &str| {
        crate::embedded::FRONTEND_ASSETS
            .iter()
            .find_map(|(path, bytes)| (*path == name).then_some(*bytes))
    };
    let entry_point =
        embedded_asset("index.html").expect("the embedded frontend has an entry point");
    let script = crate::embedded::FRONTEND_ASSETS
        .iter()
        .map(|(path, _)| *path)
        .find(|path| path.ends_with(".js"))
        .expect("the embedded frontend has a script");

    for path in ["", "deep/link"] {
        let response = application
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/{path}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/html; charset=utf-8",
            "{path}"
        );
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], entry_point, "{path}");
    }

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/{script}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );

    let response = application
        .oneshot(
            Request::builder()
                .uri("/api/v1/not-a-route")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
}
