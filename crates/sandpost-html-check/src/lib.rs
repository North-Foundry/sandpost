//! Email client compatibility check for HTML bodies, based on caniemail.com data.
//!
//! The check is I/O-free: it parses the HTML and its CSS, detects the caniemail features in use,
//! and reports, per email client, whether every detected feature is supported. Linked stylesheets
//! are counted but never downloaded. The embedded data is MIT-licensed (see
//! `data/CANIEMAIL-LICENSE`).

mod database;
mod detection;
mod error;
mod features;
mod report;
mod selector;
mod stylesheet;

#[cfg(test)]
mod tests;

pub use database::SupportLevel;
pub use error::HtmlCheckError;
pub use report::{
    ClientSupport, CompatibilityReport, CompatibilitySummary, DataSource, EmailClient,
    FeatureReport, WeightedScore,
};

/// Largest HTML body that is analysed, in bytes.
pub const HTML_SIZE_LIMIT: usize = 4 * 1024 * 1024;

/**
 * Check an HTML body against the embedded caniemail data.
 *
 * Bodies larger than [`HTML_SIZE_LIMIT`] are refused rather than truncated, because a partial
 * document would report misleading results. The work is CPU-bound; async callers should run it on a
 * blocking thread.
 */
pub fn check_html(html: &str) -> Result<CompatibilityReport, HtmlCheckError> {
    if html.len() > HTML_SIZE_LIMIT {
        return Err(HtmlCheckError::TooLarge {
            size: html.len(),
            limit: HTML_SIZE_LIMIT,
        });
    }
    let detection = detection::detect(html);
    Ok(report::build_report(
        &detection,
        database::support_database(),
    ))
}
