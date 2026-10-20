//! I/O-free reading of a message's HTML for `open` and `click`.
//!
//! Parsing uses html5ever through `scraper`, so malformed markup is repaired the way browsers
//! repair it instead of failing. Nothing here fetches anything or runs script; callers request the
//! returned URLs themselves under their own network policy. Parsing is CPU-bound, so asynchronous
//! callers should run these functions on a blocking thread.
//!
//! URL policy shared by images and links: references are trimmed, then parsed as absolute URLs.
//! A relative reference is resolved against the document's first `<base href>` when that is an
//! absolute `http` or `https` URL; without one, protocol-relative references (`//host/path`) use
//! `https`, and other relative references cannot be resolved because an email has no address of
//! its own. Only `http` and `https` URLs with a host are accepted; URLs carrying a user name or
//! password are refused so credentials are never sent anywhere. Fragments are removed because
//! they never reach a server, and the WHATWG serialization of the result is used.
use crate::syntax::LinkSelector;
use sandpost_query::glob_matches;
use scraper::{ElementRef, Html, Selector};
use std::collections::HashSet;
use std::sync::OnceLock;
use url::Url;

/// Longest URL requested, in bytes.
pub const MAXIMUM_URL_LENGTH: usize = 2048;

/// Most distinct remote images one `open` requests; later images are counted, not requested.
pub const MAXIMUM_REMOTE_IMAGES: usize = 32;

/// Longest link text kept for reports, in characters.
const MAXIMUM_REPORTED_TEXT: usize = 200;

/// Why a reference cannot be requested.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UrlRejection {
    #[error("the URL is empty")]
    Empty,
    #[error("the URL is longer than {MAXIMUM_URL_LENGTH} bytes")]
    TooLong,
    #[error("the relative URL cannot be resolved because the message has no absolute <base href>")]
    Relative,
    #[error("the {0}: scheme is not supported; only http and https URLs are requested")]
    UnsupportedScheme(String),
    #[error("the URL is not valid")]
    Invalid,
    #[error("URLs containing a user name or password are never requested")]
    EmbeddedCredentials,
}

/**
 * Resolve a reference from the message to an absolute, requestable `http` or `https` URL.
 *
 * See the module documentation for the exact policy.
 */
pub fn resolve_url(base: Option<&Url>, reference: &str) -> Result<String, UrlRejection> {
    let reference = reference.trim_matches(|character: char| character.is_ascii_whitespace());
    if reference.is_empty() {
        return Err(UrlRejection::Empty);
    }
    if reference.len() > MAXIMUM_URL_LENGTH {
        return Err(UrlRejection::TooLong);
    }
    let mut url = match Url::parse(reference) {
        Ok(url) => url,
        Err(url::ParseError::RelativeUrlWithoutBase) => match base {
            Some(base) => base.join(reference).map_err(|_| UrlRejection::Invalid)?,
            None if reference.starts_with("//") => {
                Url::parse(&format!("https:{reference}")).map_err(|_| UrlRejection::Invalid)?
            }
            None => return Err(UrlRejection::Relative),
        },
        Err(_) => return Err(UrlRejection::Invalid),
    };
    if !matches!(url.scheme(), "http" | "https") {
        return Err(UrlRejection::UnsupportedScheme(url.scheme().to_owned()));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(UrlRejection::Invalid);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(UrlRejection::EmbeddedCredentials);
    }
    url.set_fragment(None);
    let resolved: String = url.into();
    if resolved.len() > MAXIMUM_URL_LENGTH {
        return Err(UrlRejection::TooLong);
    }
    Ok(resolved)
}

/// The remote images a mail client would load when displaying a message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteImages {
    /// Distinct requestable URLs in document order, at most [`MAXIMUM_REMOTE_IMAGES`].
    pub urls: Vec<String>,
    /// Distinct requestable URLs beyond the limit, which are not requested.
    pub omitted: usize,
    /// `cid:` and `data:` images, which clients display from the message itself.
    pub embedded: usize,
    /// References that cannot be requested: other schemes, unresolvable, invalid, or carrying
    /// credentials.
    pub rejected: usize,
}

/**
 * Find the remote images of a message's HTML.
 *
 * Image references are read, in document order, from `<img src>` (or the first `srcset`
 * candidate of an `<img>` without `src`), `<input type="image" src>`, and `background`
 * attributes such as `<td background>`. Stylesheets and CSS `url()` values are not read.
 */
pub fn remote_images(html: &str) -> RemoteImages {
    let mut images = RemoteImages::default();
    if html.trim().is_empty() {
        return images;
    }
    let document = Html::parse_document(html);
    let base = document_base(&document);
    let mut seen = HashSet::new();
    for element in document.select(image_selector()) {
        let value = element.value();
        let mut references = Vec::new();
        match value.name() {
            "img" => match value.attr("src").filter(|source| !source.trim().is_empty()) {
                Some(source) => references.push(source),
                None => references.extend(value.attr("srcset").and_then(first_srcset_candidate)),
            },
            "input"
                if value
                    .attr("type")
                    .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("image")) =>
            {
                references.extend(value.attr("src"));
            }
            _ => {}
        }
        references.extend(value.attr("background"));
        for reference in references {
            let trimmed = reference.trim();
            let lowercase = trimmed.get(..5).unwrap_or(trimmed).to_ascii_lowercase();
            if lowercase.starts_with("cid:") || lowercase.starts_with("data:") {
                images.embedded += 1;
                continue;
            }
            match resolve_url(base.as_ref(), trimmed) {
                Ok(url) if seen.insert(url.clone()) => {
                    if images.urls.len() < MAXIMUM_REMOTE_IMAGES {
                        images.urls.push(url);
                    } else {
                        images.omitted += 1;
                    }
                }
                Ok(_) => {}
                Err(UrlRejection::Empty) => {}
                Err(_) => images.rejected += 1,
            }
        }
    }
    images
}

/// The link a `click` selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedLink {
    /// The resolved URL that will be requested.
    pub url: String,
    /// The `href` attribute as written in the message.
    pub href: String,
    /// The link's visible text with whitespace collapsed, shortened for reports.
    pub text: String,
}

/// Why no link could be followed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkSelectionError {
    #[error("the message has no HTML body to click in")]
    NoHtml,
    #[error("no element matches the CSS selector")]
    NoCssMatch,
    #[error(
        "the CSS selector matched {0} element(s), but none is or is inside a link with an href"
    )]
    NotALink(usize),
    #[error("none of the message's {0} link(s) has matching text")]
    NoTextMatch(usize),
    #[error("none of the message's {0} link(s) has a matching URL")]
    NoHrefMatch(usize),
    #[error("the selected link cannot be followed: {reason}")]
    Unfollowable {
        href: String,
        text: String,
        reason: UrlRejection,
    },
}

/**
 * Select the link a `click` follows and resolve its URL.
 *
 * Links are `<a href>` and `<area href>` elements, considered in document order:
 *
 * - `link("css")` takes the first matched element that is a link, or failing that whose closest
 *   ancestor is a link, because clicking that element would follow the ancestor.
 * - `link(text: "...")` matches the link's visible text (descendant text and image `alt` text;
 *   an `<area>` uses its `alt`) after collapsing whitespace, as a whole-value glob that ignores
 *   ASCII case. Without `*` or `?` the text must match exactly.
 * - `link(href: "...")` matches the resolved absolute URL (or the trimmed attribute when it cannot
 *   be resolved) as a case-sensitive whole-value glob, the `matches` operator's semantics.
 *
 * Selecting a link whose URL cannot be requested is an error rather than a fall-through to the
 * next link, so a script never follows a different link than the one it described.
 */
pub fn select_link(
    html: &str,
    selector: &LinkSelector,
) -> Result<SelectedLink, LinkSelectionError> {
    if html.trim().is_empty() {
        return Err(LinkSelectionError::NoHtml);
    }
    let document = Html::parse_document(html);
    let base = document_base(&document);
    let element = match selector {
        LinkSelector::Css(css) => {
            let mut matched = 0;
            let mut found = None;
            for element in document.select(css.selector()) {
                matched += 1;
                if let Some(link) = enclosing_link(element) {
                    found = Some(link);
                    break;
                }
            }
            match found {
                Some(link) => link,
                None if matched == 0 => return Err(LinkSelectionError::NoCssMatch),
                None => return Err(LinkSelectionError::NotALink(matched)),
            }
        }
        LinkSelector::Text(pattern) => {
            let pattern = collapse_whitespace(pattern);
            let links: Vec<_> = document.select(link_selector()).collect();
            links
                .iter()
                .copied()
                .find(|link| glob_matches(&visible_text(*link), &pattern, true))
                .ok_or(LinkSelectionError::NoTextMatch(links.len()))?
        }
        LinkSelector::Href(pattern) => {
            let links: Vec<_> = document.select(link_selector()).collect();
            links
                .iter()
                .copied()
                .find(|link| {
                    let href = link.value().attr("href").unwrap_or_default();
                    match resolve_url(base.as_ref(), href) {
                        Ok(url) => glob_matches(&url, pattern, false),
                        Err(_) => glob_matches(href.trim(), pattern, false),
                    }
                })
                .ok_or(LinkSelectionError::NoHrefMatch(links.len()))?
        }
    };
    let href = element.value().attr("href").unwrap_or_default().to_owned();
    let text: String = visible_text(element)
        .chars()
        .take(MAXIMUM_REPORTED_TEXT)
        .collect();
    match resolve_url(base.as_ref(), &href) {
        Ok(url) => Ok(SelectedLink { url, href, text }),
        Err(reason) => Err(LinkSelectionError::Unfollowable { href, text, reason }),
    }
}

/**
 * Return the element itself when it is a link, otherwise its closest link ancestor.
 */
fn enclosing_link(element: ElementRef<'_>) -> Option<ElementRef<'_>> {
    std::iter::once(element)
        .chain(element.ancestors().filter_map(ElementRef::wrap))
        .find(|candidate| is_link(*candidate))
}

/**
 * Report whether an element is an `<a>` or `<area>` with an `href`.
 */
fn is_link(element: ElementRef<'_>) -> bool {
    matches!(element.value().name(), "a" | "area") && element.value().attr("href").is_some()
}

/**
 * Return a link's visible text: descendant text plus image `alt` text, or an area's `alt`.
 */
fn visible_text(link: ElementRef<'_>) -> String {
    if link.value().name() == "area" {
        return collapse_whitespace(link.value().attr("alt").unwrap_or_default());
    }
    let mut parts: Vec<&str> = link.text().collect();
    for image in link.select(image_alternative_selector()) {
        parts.extend(image.value().attr("alt"));
    }
    collapse_whitespace(&parts.join(" "))
}

/**
 * Trim a string and collapse every run of whitespace into one space.
 */
fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/**
 * Return the URL of the document's first `<base href>` when it is absolute `http` or `https`.
 */
fn document_base(document: &Html) -> Option<Url> {
    let href = document
        .select(base_selector())
        .next()?
        .value()
        .attr("href")?;
    let url = Url::parse(href.trim()).ok()?;
    matches!(url.scheme(), "http" | "https").then_some(url)
}

/**
 * Return the URL of the first candidate of a `srcset` attribute.
 */
fn first_srcset_candidate(srcset: &str) -> Option<&str> {
    srcset
        .split(',')
        .next()?
        .split_whitespace()
        .next()
        .filter(|candidate| !candidate.is_empty())
}

/**
 * Return a lazily compiled selector.
 */
fn cached(cell: &'static OnceLock<Selector>, source: &'static str) -> &'static Selector {
    cell.get_or_init(|| Selector::parse(source).expect("valid built-in selector"))
}

/**
 * Select elements that can reference a remote image.
 */
fn image_selector() -> &'static Selector {
    static SELECTOR: OnceLock<Selector> = OnceLock::new();
    cached(&SELECTOR, "img, input[src], [background]")
}

/**
 * Select links that navigate to another document.
 */
fn link_selector() -> &'static Selector {
    static SELECTOR: OnceLock<Selector> = OnceLock::new();
    cached(&SELECTOR, "a[href], area[href]")
}

/**
 * Select images whose alternative text is part of a link's visible text.
 */
fn image_alternative_selector() -> &'static Selector {
    static SELECTOR: OnceLock<Selector> = OnceLock::new();
    cached(&SELECTOR, "img[alt]")
}

/**
 * Select the document base element.
 */
fn base_selector() -> &'static Selector {
    static SELECTOR: OnceLock<Selector> = OnceLock::new();
    cached(&SELECTOR, "base[href]")
}
