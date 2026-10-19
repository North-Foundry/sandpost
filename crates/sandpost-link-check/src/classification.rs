//! Verdicts derived from final HTTP status codes.
use sandpost_core::LinkOutcome;

/**
 * Classify the status code of the final response, after redirects have been followed.
 *
 * Success is healthy and server errors are server errors. Client errors are broken, except
 * authentication, proxy authentication, access refusals and rate limits (401, 403, 407, 429):
 * these commonly answer automated requests to working pages, so the verdict is inconclusive.
 * Informational codes, redirects without a location and non-standard codes such as 999 are
 * inconclusive too.
 */
pub fn classify_status(code: u16) -> LinkOutcome {
    match code {
        200..=299 => LinkOutcome::Healthy,
        401 | 403 | 407 | 429 => LinkOutcome::Inconclusive,
        400..=499 => LinkOutcome::Broken,
        500..=599 => LinkOutcome::ServerError,
        _ => LinkOutcome::Inconclusive,
    }
}

/**
 * Return the standard reason phrase of a status code, such as `Not Found` for 404.
 *
 * Servers may send any phrase; the registered one is reported so results read consistently.
 */
pub fn status_reason(code: u16) -> Option<&'static str> {
    hyper::StatusCode::from_u16(code)
        .ok()
        .and_then(|status| status.canonical_reason())
}
