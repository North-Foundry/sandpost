//! Discovery of distinct absolute HTTP and HTTPS links in message bodies.
use scraper::{Html, Selector};
use std::collections::HashSet;
use std::sync::OnceLock;
use url::Url;

/// Largest number of distinct links verified for one message; later links are counted only.
pub const MAXIMUM_LINKS: usize = 200;

/// Longest link accepted, in bytes; longer values are ignored as unverifiable.
pub const MAXIMUM_URL_LENGTH: usize = 2048;

/// Distinct links in first-appearance order, plus the number beyond [`MAXIMUM_LINKS`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedLinks {
    pub urls: Vec<String>,
    pub omitted: usize,
}

/**
 * Normalize one link exactly as browsers parse an absolute URL, or reject it.
 *
 * Only absolute `http` and `https` URLs with a host are accepted. Surrounding whitespace is
 * ignored, the fragment is removed because it never reaches the server, and the WHATWG
 * serialization becomes the stored identity, so clients can match rendered `href` values by
 * applying the same normalization.
 */
pub fn normalize_link(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAXIMUM_URL_LENGTH {
        return None;
    }
    let mut url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
        return None;
    }
    url.set_fragment(None);
    let normalized: String = url.into();
    (normalized.len() <= MAXIMUM_URL_LENGTH).then_some(normalized)
}

/**
 * Collect the distinct links of an HTML body and a plain-text body.
 *
 * HTML anchors and image-map areas come first in document order, followed by links that occur
 * only in the text. Relative, `mailto:`, `tel:`, `javascript:` and other non-HTTP references are
 * ignored. The work is CPU-bound; async callers should run it on a blocking thread.
 */
pub fn extract_links(html: &str, text: &str) -> ExtractedLinks {
    let mut seen = HashSet::new();
    let mut links = ExtractedLinks::default();
    let mut accept = |candidate: &str| {
        let Some(url) = normalize_link(candidate) else {
            return;
        };
        if !seen.insert(url.clone()) {
            return;
        }
        if links.urls.len() < MAXIMUM_LINKS {
            links.urls.push(url);
        } else {
            links.omitted += 1;
        }
    };
    if !html.trim().is_empty() {
        let document = Html::parse_document(html);
        for element in document.select(link_selector()) {
            if let Some(href) = element.value().attr("href") {
                accept(href);
            }
        }
    }
    for candidate in text_links(text) {
        accept(candidate);
    }
    links
}

/**
 * Return the compiled selector for elements whose `href` navigates to another document.
 */
fn link_selector() -> &'static Selector {
    static SELECTOR: OnceLock<Selector> = OnceLock::new();
    SELECTOR.get_or_init(|| Selector::parse("a[href], area[href]").expect("valid link selector"))
}

/**
 * Find absolute `http://` and `https://` URLs in plain text.
 *
 * A URL ends at whitespace, angle brackets, quotes or backticks. Trailing sentence punctuation is
 * removed, as is a closing bracket that has no opening partner inside the URL, so prose such as
 * `(see https://example.test/a).` yields `https://example.test/a`.
 */
fn text_links(text: &str) -> impl Iterator<Item = &str> {
    let lowercase = text.to_ascii_lowercase();
    let mut starts = Vec::new();
    let mut offset = 0;
    while let Some(position) = lowercase[offset..].find("http") {
        let start = offset + position;
        let rest = &lowercase[start..];
        if (rest.starts_with("http://") || rest.starts_with("https://"))
            && !text[..start]
                .chars()
                .next_back()
                .is_some_and(|previous| previous.is_ascii_alphanumeric())
        {
            starts.push(start);
        }
        offset = start + 4;
    }
    starts.into_iter().filter_map(move |start| {
        let candidate = &text[start..];
        let end = candidate
            .find(|character: char| {
                character.is_whitespace()
                    || matches!(character, '<' | '>' | '"' | '\'' | '`')
                    || character.is_control()
            })
            .unwrap_or(candidate.len());
        // Longer runs cannot normalize to an accepted link; skip them before trimming.
        if end > MAXIMUM_URL_LENGTH * 2 {
            return None;
        }
        let trimmed = trim_trailing_punctuation(&candidate[..end]);
        (trimmed.len() > "https://".len()).then_some(trimmed)
    })
}

/**
 * Remove punctuation that ends the surrounding sentence rather than the URL.
 */
fn trim_trailing_punctuation(mut candidate: &str) -> &str {
    loop {
        let Some(last) = candidate.chars().next_back() else {
            return candidate;
        };
        let unbalanced = match last {
            ')' => candidate.matches('(').count() < candidate.matches(')').count(),
            ']' => candidate.matches('[').count() < candidate.matches(']').count(),
            '}' => candidate.matches('{').count() < candidate.matches('}').count(),
            '.' | ',' | ';' | ':' | '!' | '?' | '*' => true,
            _ => false,
        };
        if !unbalanced {
            return candidate;
        }
        candidate = &candidate[..candidate.len() - last.len_utf8()];
    }
}
