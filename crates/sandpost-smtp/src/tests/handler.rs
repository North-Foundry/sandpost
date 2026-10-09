//! Display and error-trait behavior of the public handler error types.
use crate::{AuthenticationError, DeliveryError};

/// The public error types describe themselves for logs.
#[test]
fn handler_errors_describe_themselves() {
    assert_eq!(
        AuthenticationError::Unavailable("storage offline".into()).to_string(),
        "credentials could not be verified: storage offline"
    );
    assert_eq!(
        DeliveryError::Rejected("revoked".into()).to_string(),
        "message rejected: revoked"
    );
    assert_eq!(
        DeliveryError::Temporary("disk full".into()).to_string(),
        "message could not be stored: disk full"
    );
    let error: &dyn std::error::Error = &DeliveryError::Temporary("disk full".into());
    assert!(error.source().is_none());
}
