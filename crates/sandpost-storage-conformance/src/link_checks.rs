//! Shared link check result persistence checks.
use sandpost_core::{
    LinkCheckErrorKind, LinkCheckResult, LinkOutcome, MessageIdentifier, MessageLinkCheck,
};
use sandpost_storage::{Storage, StorageError};

use crate::ConformanceFailure;
use crate::support::{failure, simple_message, unwrap_storage, verify, verify_equal};

/**
 * Build one stored result for a URL with the supplied outcome and completion time.
 */
fn result(url: &str, outcome: LinkOutcome, checked_at: i64) -> LinkCheckResult {
    LinkCheckResult {
        url: url.to_owned(),
        outcome,
        status_code: match outcome {
            LinkOutcome::Healthy => Some(200),
            LinkOutcome::Broken => Some(404),
            LinkOutcome::ServerError => Some(503),
            LinkOutcome::Inconclusive => Some(403),
            LinkOutcome::Failed => None,
        },
        final_url: None,
        redirect_count: 0,
        error_kind: (outcome == LinkOutcome::Failed).then_some(LinkCheckErrorKind::Timeout),
        error_message: (outcome == LinkOutcome::Failed).then(|| "timed out".to_owned()),
        checked_at,
    }
}

/**
 * Verify storing, replacing, ordering, refusing, and collecting a message's link check.
 *
 * Results round-trip in message order with every field, a replacement removes links absent
 * from the newer check, an older replacement and a duplicate URL leave the stored check intact,
 * unknown messages fail with NotFound, and physical message deletion removes the check.
 */
pub async fn link_check_results(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "link_check_results";
    let message = simple_message("link check");
    unwrap_storage(
        CHECK,
        "insert_message",
        storage.insert_message(&message).await,
    )?;
    let identifier = message.identifier;
    verify_equal(
        CHECK,
        "unchecked message",
        unwrap_storage(
            CHECK,
            "get_message_link_check",
            storage.get_message_link_check(identifier).await,
        )?,
        None,
    )?;

    let mut redirected = result("https://example.test/moved", LinkOutcome::Healthy, 1_000);
    redirected.final_url = Some("https://example.test/destination".to_owned());
    redirected.redirect_count = 2;
    let first = MessageLinkCheck {
        message_identifier: identifier,
        checked_at: 1_000,
        omitted_link_count: 3,
        links: vec![
            result("https://example.test/z-first", LinkOutcome::Broken, 999),
            redirected,
            result("https://example.test/a-last", LinkOutcome::Failed, 1_000),
        ],
    };
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "replace_message_link_check",
            storage.replace_message_link_check(&first).await,
        )?,
        "a first check must be stored",
    )?;
    verify_equal(
        CHECK,
        "stored first check",
        unwrap_storage(
            CHECK,
            "get_message_link_check",
            storage.get_message_link_check(identifier).await,
        )?,
        Some(first.clone()),
    )?;

    let second = MessageLinkCheck {
        message_identifier: identifier,
        checked_at: 2_000,
        omitted_link_count: 0,
        links: vec![
            result("https://example.test/a-last", LinkOutcome::Healthy, 2_000),
            result("https://example.test/new", LinkOutcome::ServerError, 2_000),
        ],
    };
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "replace_message_link_check with a newer check",
            storage.replace_message_link_check(&second).await,
        )?,
        "a newer check must replace the stored check",
    )?;
    verify_equal(
        CHECK,
        "replaced check",
        unwrap_storage(
            CHECK,
            "get_message_link_check",
            storage.get_message_link_check(identifier).await,
        )?,
        Some(second.clone()),
    )?;

    let stale = MessageLinkCheck {
        checked_at: 1_500,
        ..first.clone()
    };
    verify(
        CHECK,
        !unwrap_storage(
            CHECK,
            "replace_message_link_check with an older check",
            storage.replace_message_link_check(&stale).await,
        )?,
        "an older check must not replace a newer stored check",
    )?;
    let duplicate = MessageLinkCheck {
        checked_at: 3_000,
        links: vec![
            result("https://example.test/same", LinkOutcome::Healthy, 3_000),
            result("https://example.test/same", LinkOutcome::Broken, 3_000),
        ],
        ..second.clone()
    };
    match storage.replace_message_link_check(&duplicate).await {
        Err(StorageError::ConstraintViolation(_)) => {}
        other => {
            return Err(failure(
                CHECK,
                format!("a duplicate URL must be a constraint violation, got {other:?}"),
            ));
        }
    }
    verify_equal(
        CHECK,
        "check after refused replacements",
        unwrap_storage(
            CHECK,
            "get_message_link_check",
            storage.get_message_link_check(identifier).await,
        )?,
        Some(second),
    )?;

    let unknown = MessageLinkCheck {
        message_identifier: MessageIdentifier::new(),
        ..first
    };
    match storage.replace_message_link_check(&unknown).await {
        Err(StorageError::NotFound) => {}
        other => {
            return Err(failure(
                CHECK,
                format!("an unknown message must fail with NotFound, got {other:?}"),
            ));
        }
    }

    unwrap_storage(
        CHECK,
        "delete_message",
        storage.delete_message(identifier).await,
    )?;
    verify_equal(
        CHECK,
        "check after physical message deletion",
        unwrap_storage(
            CHECK,
            "get_message_link_check",
            storage.get_message_link_check(identifier).await,
        )?,
        None,
    )?;
    Ok(())
}
