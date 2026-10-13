//! Manual folder HTTP contracts and placement across list, search, and direct reads.
use super::*;
use serde_json::{Value, json};

/// Send a JSON request through the authenticated router and retain its decoded response.
async fn request(
    application: &Router,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
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

/// Folder placement excludes baseline lists while retaining search and direct mail access.
#[tokio::test]
async fn manual_folder_http_crud_and_placement_preserve_mail() {
    let storage = memory_storage().await;
    let identifier = MessageIdentifier::new();
    storage
        .insert_message(&Message {
            identifier,
            facts: MessageFacts {
                subject: "Production error".into(),
                ..Default::default()
            },
            raw_message: b"Subject: Production error\r\n\r\nbody".to_vec(),
            attachments: vec![],
        })
        .await
        .unwrap();
    let application = initialized_router(storage.clone()).await;
    let (status, folder) = request(
        &application,
        "POST",
        "/api/v1/folders",
        json!({ "name": "To Review", "exposedAccounts": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let folder_identifier = folder["identifier"].as_str().unwrap();
    assert_eq!(folder["exposed_accounts"], json!([]));
    let (status, result) = request(
        &application,
        "POST",
        "/api/v1/messages/placement",
        json!({ "identifiers": [identifier], "folderIdentifier": folder_identifier }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["moved"], 1);
    assert_eq!(
        request(&application, "GET", "/api/v1/messages", Value::Null)
            .await
            .1,
        json!([])
    );
    let (status, global_results) =
        request(&application, "GET", "/api/v1/messages?q=true", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        global_results[0]["id"],
        identifier.to_string(),
        "global canonical search retains filed mail"
    );
    let (status, rows) = request(
        &application,
        "GET",
        &format!("/api/v1/folders/{folder_identifier}/messages"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rows[0]["id"], identifier.to_string());
    assert_eq!(
        request(
            &application,
            "GET",
            &format!("/api/v1/messages/{identifier}"),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    let (status, renamed) = request(
        &application,
        "PUT",
        &format!("/api/v1/folders/{folder_identifier}"),
        json!({ "name": "Reviewed", "exposedAccounts": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["identifier"], folder_identifier);
    assert_eq!(renamed["name"], "Reviewed");
    assert_eq!(
        request(
            &application,
            "DELETE",
            &format!("/api/v1/folders/{folder_identifier}"),
            Value::Null
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(&application, "GET", "/api/v1/messages", Value::Null)
            .await
            .1[0]["id"],
        identifier.to_string()
    );
    assert!(storage.get_message(identifier).await.unwrap().is_some());
}

/// Missing placement destinations are rejected without changing a message's existing folder.
#[tokio::test]
async fn placement_requires_destination_but_accepts_null_and_folder_identifier() {
    let storage = memory_storage().await;
    let identifier = MessageIdentifier::new();
    storage
        .insert_message(&Message {
            identifier,
            facts: MessageFacts::default(),
            raw_message: b"Subject: placement\r\n\r\nbody".to_vec(),
            attachments: vec![],
        })
        .await
        .unwrap();
    let application = initialized_router(storage).await;
    let (status, folder) = request(
        &application,
        "POST",
        "/api/v1/folders",
        json!({ "name": "Filed", "exposedAccounts": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let folder_identifier = folder["identifier"].as_str().unwrap();

    assert_eq!(
        request(
            &application,
            "POST",
            "/api/v1/messages/placement",
            json!({ "identifiers": [identifier], "folderIdentifier": folder_identifier }),
        )
        .await
        .0,
        StatusCode::OK,
        "a valid folder identifier is accepted"
    );
    assert_eq!(
        request(
            &application,
            "POST",
            "/api/v1/messages/placement",
            json!({ "identifiers": [identifier] }),
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an omitted destination is rejected"
    );
    assert_eq!(
        request(&application, "GET", "/api/v1/messages", Value::Null)
            .await
            .1,
        json!([]),
        "the rejected request leaves the message filed"
    );
    assert_eq!(
        request(
            &application,
            "GET",
            &format!("/api/v1/folders/{folder_identifier}/messages"),
            Value::Null,
        )
        .await
        .1[0]["id"],
        identifier.to_string(),
        "the existing folder placement is preserved"
    );

    assert_eq!(
        request(
            &application,
            "POST",
            "/api/v1/messages/placement",
            json!({ "identifiers": [identifier], "folderIdentifier": null }),
        )
        .await
        .0,
        StatusCode::OK,
        "explicit null restores baseline placement"
    );
    assert_eq!(
        request(&application, "GET", "/api/v1/messages", Value::Null)
            .await
            .1[0]["id"],
        identifier.to_string()
    );
}

/// Folder moves reject overlong batches and foreign exposure without partially saving state.
#[tokio::test]
async fn manual_folder_http_validation_rejects_invalid_state() {
    let application = initialized_router(memory_storage().await).await;
    let account = sandpost_core::ImapAccountIdentifier::new();
    assert_eq!(
        request(
            &application,
            "POST",
            "/api/v1/folders",
            json!({ "name": "Bad exposure", "exposedAccounts": [account] })
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&application, "GET", "/api/v1/folders", Value::Null)
            .await
            .1,
        json!([])
    );
    let identifiers = vec![MessageIdentifier::new(); 257];
    assert_eq!(
        request(
            &application,
            "POST",
            "/api/v1/messages/placement",
            json!({ "identifiers": identifiers, "folderIdentifier": null })
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &application,
            "GET",
            &format!(
                "/api/v1/folders/{}/messages",
                sandpost_core::ImapFolderIdentifier::new()
            ),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

/// Every manual folder and placement route requires authentication, including mutation bodies.
#[tokio::test]
async fn manual_folder_and_placement_routes_require_a_session() {
    let storage = memory_storage().await;
    let (events, _) = tokio::sync::broadcast::channel(16);
    let (state, _) = initialized_state(storage, events).await;
    let application = router(state);
    let folder = sandpost_core::ImapFolderIdentifier::new();
    for (method, path) in [
        ("GET", "/api/v1/folders".to_owned()),
        ("POST", "/api/v1/folders".to_owned()),
        ("PUT", format!("/api/v1/folders/{folder}")),
        ("DELETE", format!("/api/v1/folders/{folder}")),
        ("GET", format!("/api/v1/folders/{folder}/messages")),
        ("POST", "/api/v1/messages/placement".to_owned()),
    ] {
        assert_eq!(
            request(&application, method, &path, json!({})).await.0,
            StatusCode::UNAUTHORIZED,
            "{method} {path}"
        );
    }
}
