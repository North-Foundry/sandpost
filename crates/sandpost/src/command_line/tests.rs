use super::*;

/// Preserve the ordinary runtime and maintenance command paths.
#[test]
fn default_runtime_and_maintenance_still_parse() {
    assert!(parse(&[]).unwrap().maintenance.is_none());
    assert_eq!(
        parse(&["search".into(), "status".into()])
            .unwrap()
            .maintenance,
        Some(MaintenanceCommand::SearchStatus),
    );
}

/// Every build profile accepts a URL and rejects malformed or repeated flags.
#[test]
fn proxy_url_flag_selects_the_frontend_origin() {
    assert!(parse(&[]).unwrap().frontend_proxy_url.is_none());
    let command = parse(&["--frontend-proxy-url".into(), "http://frontend:8080".into()]).unwrap();
    assert!(command.maintenance.is_none());
    #[cfg(debug_assertions)]
    assert!(command.frontend_proxy_file.is_none());
    assert_eq!(
        command.frontend_proxy_url.as_deref(),
        Some("http://frontend:8080"),
    );
    for arguments in [
        vec!["--frontend-proxy-url".into()],
        vec![
            "--frontend-proxy-url".into(),
            "http://127.0.0.1:5173".into(),
            "search".into(),
        ],
        vec![
            "--frontend-proxy-url".into(),
            "http://127.0.0.1:5173".into(),
            "--frontend-proxy-url".into(),
            "http://localhost:5173".into(),
        ],
    ] {
        assert!(parse(&arguments).is_err());
    }
}

/// Debug builds accept both proxy options in either order and retain each value.
#[cfg(debug_assertions)]
#[test]
fn proxy_url_and_file_flags_are_accepted_in_either_order() {
    for arguments in [
        vec![
            "--frontend-proxy-url".into(),
            "http://frontend:8080".into(),
            "--frontend-proxy-file".into(),
            "/tmp/vite-origin".into(),
        ],
        vec![
            "--frontend-proxy-file".into(),
            "/tmp/vite-origin".into(),
            "--frontend-proxy-url".into(),
            "http://frontend:8080".into(),
        ],
    ] {
        let command = parse(&arguments).unwrap();
        assert_eq!(
            command.frontend_proxy_url.as_deref(),
            Some("http://frontend:8080")
        );
        assert_eq!(
            command.frontend_proxy_file.as_deref(),
            Some(std::path::Path::new("/tmp/vite-origin"))
        );
    }
}

/// Debug builds accept one proxy file and reject malformed or repeated flags.
#[cfg(debug_assertions)]
#[test]
fn proxy_file_flag_requires_one_path() {
    let command = parse(&["--frontend-proxy-file".into(), "/tmp/vite-origin".into()]).unwrap();
    assert!(command.maintenance.is_none());
    assert!(command.frontend_proxy_url.is_none());
    assert_eq!(
        command.frontend_proxy_file.as_deref(),
        Some(std::path::Path::new("/tmp/vite-origin"))
    );
    for arguments in [
        vec!["--frontend-proxy-file".into()],
        vec!["--frontend-proxy-file".into(), "".into()],
        vec!["unexpected".into(), "--frontend-proxy-file".into()],
        vec![
            "--frontend-proxy-url".into(),
            "--frontend-proxy-file".into(),
        ],
        vec![
            "--frontend-proxy-file".into(),
            "origin".into(),
            "search".into(),
        ],
        vec![
            "--frontend-proxy-file".into(),
            "origin".into(),
            "--frontend-proxy-file".into(),
            "second-origin".into(),
        ],
    ] {
        assert!(parse(&arguments).is_err());
    }
}

/// Release builds reject the debug-only proxy file option.
#[cfg(not(debug_assertions))]
#[test]
fn proxy_file_flag_is_rejected_in_release_builds() {
    assert!(parse(&["--frontend-proxy-file".into(), "/tmp/vite-origin".into()]).is_err());
}

/// Read a trimmed origin after its file appears following the initial missing-file poll.
#[cfg(debug_assertions)]
#[tokio::test]
async fn proxy_file_reader_waits_for_publication() {
    let path = std::env::temp_dir().join(format!("sandpost-vite-origin-{}", uuid::Uuid::new_v4()));
    let published_path = path.clone();
    let publisher = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(150));
        std::fs::write(published_path, "  http://localhost:5173\n").unwrap();
    });
    assert_eq!(
        read_frontend_proxy_file(path.clone()).await.unwrap(),
        "http://localhost:5173"
    );
    publisher.join().unwrap();
    std::fs::remove_file(path).unwrap();
}

/// The former development-only spelling is rejected in every build profile.
#[test]
fn former_development_flag_is_rejected() {
    assert!(parse(&["--frontend-dev-url".into(), "http://127.0.0.1:5173".into()],).is_err());
}
