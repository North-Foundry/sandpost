//! Server-side verification of the HTTP and HTTPS links found in captured messages.
//!
//! Extraction is I/O-free: it reads `<a href>` and `<area href>` from HTML and absolute URLs from
//! plain text, deduplicates them without their fragments, and keeps their first-appearance order.
//! Verification sends `HEAD` (falling back to `GET` when `HEAD` is refused) directly from the
//! server, follows redirects manually, and classifies the final response. Every host name is
//! resolved once per request and the connection uses only the validated addresses, so private,
//! loopback, link-local and other non-public destinations are refused unless explicitly allowed,
//! including after redirects and DNS answers that mix public and private addresses.

mod address;
mod classification;
mod client;
mod extraction;

#[cfg(test)]
mod tests;

pub use address::is_public_address;
pub use classification::{classify_status, status_reason};
pub use client::{LinkCheckSettings, LinkChecker};
pub use extraction::{
    ExtractedLinks, MAXIMUM_LINKS, MAXIMUM_URL_LENGTH, extract_links, normalize_link,
};
