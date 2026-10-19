//! Feature detection over a parsed HTML document and its CSS.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use indexmap::IndexMap;
use scraper::node::Node;
use scraper::{ElementRef, Html, Selector};

use crate::features;
use crate::selector::analyse_selector_list;
use crate::stylesheet::{
    CssRule, Declaration, analyse_value, parse_declaration_list, parse_stylesheet,
};

/// Longest example text kept for one occurrence.
const EXAMPLE_LENGTH: usize = 80;

/// Examples kept per feature.
const EXAMPLES_PER_FEATURE: usize = 3;

/// Occurrences of one detected feature.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Finding {
    pub(crate) occurrences: usize,
    /// Occurrences that come from selectors which could not be checked against the document.
    pub(crate) unverified_occurrences: usize,
    /// Occurrences inside graceful contexts, such as `@media` blocks or `:hover` rules, which
    /// clients without support simply skip.
    pub(crate) graceful_occurrences: usize,
    pub(crate) examples: Vec<String>,
}

/// Detected features keyed by caniemail slug, in detection order.
#[derive(Debug, Default)]
pub(crate) struct Findings {
    entries: IndexMap<&'static str, Finding>,
    /// Depth of nested graceful contexts being analysed; occurrences inside any count as graceful.
    graceful_depth: usize,
}

impl Findings {
    /**
     * Count one occurrence of a feature, keeping a few distinct examples.
     */
    pub(crate) fn record(&mut self, slug: &'static str, example: &str) {
        let finding = self.entries.entry(slug).or_default();
        finding.occurrences += 1;
        if self.graceful_depth > 0 {
            finding.graceful_occurrences += 1;
        }
        let example = shorten(example);
        if finding.examples.len() < EXAMPLES_PER_FEATURE
            && !example.is_empty()
            && !finding.examples.contains(&example)
        {
            finding.examples.push(example);
        }
    }

    /**
     * Start or finish analysing content that clients without support skip without breaking the
     * message, when `graceful` is true; calls must be paired.
     */
    fn enter_graceful(&mut self, graceful: bool) {
        if graceful {
            self.graceful_depth += 1;
        }
    }

    /**
     * Finish a context started by [`Findings::enter_graceful`] with the same `graceful` value.
     */
    fn leave_graceful(&mut self, graceful: bool) {
        if graceful {
            self.graceful_depth -= 1;
        }
    }

    /**
     * Count an occurrence that comes from a selector which could not be checked against the document.
     */
    fn record_unverified(&mut self, slug: &'static str, example: &str) {
        self.record(slug, example);
        self.entries.entry(slug).or_default().unverified_occurrences += 1;
    }

    /**
     * Iterate the detected features in detection order.
     */
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&'static str, &Finding)> {
        self.entries.iter().map(|(slug, finding)| (*slug, finding))
    }

    /**
     * Return the finding of a feature, if detected.
     */
    #[cfg(test)]
    pub(crate) fn get(&self, slug: &str) -> Option<&Finding> {
        self.entries.get(slug)
    }
}

/// Everything detected in one HTML document.
#[derive(Debug, Default)]
pub(crate) struct Detection {
    pub(crate) findings: Findings,
    pub(crate) external_stylesheets: usize,
    /// Elements inside `<body>`.
    pub(crate) element_count: usize,
}

/**
 * Detect the caniemail features used by an HTML document.
 */
pub(crate) fn detect(html: &str) -> Detection {
    let document = Html::parse_document(html);
    let mut detection = Detection::default();
    detect_markup(&document, html, &mut detection);
    detect_images(&document, &mut detection.findings);
    detect_styles(&document, &mut detection.findings);
    static BODY_ELEMENTS: OnceLock<Selector> = OnceLock::new();
    detection.element_count = document
        .select(BODY_ELEMENTS.get_or_init(|| static_selector("body *")))
        .count();
    detection
}

/**
 * Return the compiled markup detectors, compiling them on first use.
 *
 * Selectors that fail to compile are skipped; the crate tests require all of them to compile.
 */
fn markup_detectors() -> &'static [(&'static str, Selector)] {
    static DETECTORS: OnceLock<Vec<(&'static str, Selector)>> = OnceLock::new();
    DETECTORS.get_or_init(|| {
        features::MARKUP_DETECTORS
            .iter()
            .filter_map(|(slug, selector)| Some((*slug, Selector::parse(selector).ok()?)))
            .collect()
    })
}

/**
 * Compile a selector known to be valid.
 */
fn static_selector(source: &'static str) -> Selector {
    Selector::parse(source).expect("built-in selector is valid")
}

/**
 * Detect element, attribute, comment, doctype and document-structure features.
 */
fn detect_markup(document: &Html, html: &str, detection: &mut Detection) {
    for (slug, selector) in markup_detectors() {
        for element in document.select(selector) {
            detection.findings.record(slug, &start_tag(element));
        }
    }
    // The parser always creates a body, so only an explicit tag in the source counts.
    if contains_ignoring_ascii_case(html, "<body") {
        detection.findings.record(features::HTML_BODY, "<body>");
    }
    for node in document.tree.root().descendants() {
        match node.value() {
            Node::Doctype(doctype)
                if doctype.name().eq_ignore_ascii_case("html")
                    && doctype.public_id().is_empty() =>
            {
                detection
                    .findings
                    .record(features::HTML_DOCTYPE, "<!DOCTYPE html>");
            }
            Node::Comment(comment) => {
                detection
                    .findings
                    .record(features::HTML_COMMENTS, &format!("<!--{}-->", &**comment));
            }
            _ => {}
        }
    }
    let root = document.root_element();
    if root
        .value()
        .attrs()
        .any(|(name, _)| matches!(name, "⚡4email" | "amp4email" | "⚡"))
    {
        detection
            .findings
            .record(features::HTML_AMP, "<html ⚡4email>");
    }
    static STYLESHEET_LINKS: OnceLock<Selector> = OnceLock::new();
    let links = STYLESHEET_LINKS.get_or_init(|| static_selector("link[rel~='stylesheet' i]"));
    detection.external_stylesheets = document.select(links).count();
}

/**
 * Detect image formats used by `<img>`, `<picture>` sources and `background` attributes.
 */
fn detect_images(document: &Html, findings: &mut Findings) {
    static IMAGE_SOURCES: OnceLock<Selector> = OnceLock::new();
    let selector =
        IMAGE_SOURCES.get_or_init(|| static_selector("img, picture source, [background]"));
    for element in document.select(selector) {
        let element = element.value();
        if let Some(source) = element.attr("src").filter(|_| element.name() == "img") {
            record_image_url(source, findings);
        }
        if let Some(source_set) = element.attr("srcset") {
            for candidate in source_set_candidates(source_set) {
                record_image_url(candidate, findings);
            }
        }
        if let Some(media_type) = element.attr("type").filter(|_| element.name() == "source")
            && let Some(slug) =
                features::image_media_type_feature(&media_type.trim().to_ascii_lowercase())
        {
            findings.record(slug, &format!("<source type=\"{media_type}\">"));
        }
        if let Some(background) = element.attr("background") {
            record_image_url(background, findings);
        }
    }
}

/**
 * Split a `srcset` attribute into its candidate URLs; a `data:` URL is taken whole because it may
 * contain commas.
 */
fn source_set_candidates(source_set: &str) -> Vec<&str> {
    let source_set = source_set.trim();
    if source_set.starts_with("data:") {
        return source_set.split_whitespace().next().into_iter().collect();
    }
    source_set
        .split(',')
        .filter_map(|candidate| candidate.split_whitespace().next())
        .collect()
}

/**
 * Record the image format of a URL, judged from a `data:` media type or the extension of its path
 * without query or fragment.
 */
fn record_image_url(url: &str, findings: &mut Findings) {
    let url = url.trim();
    if let Some(data) = url.strip_prefix("data:") {
        let media_type = data
            .split([';', ','])
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let example = format!("data:{media_type}…");
        findings.record(features::IMAGE_BASE64, &example);
        if let Some(slug) = features::image_media_type_feature(&media_type) {
            findings.record(slug, &example);
        }
        return;
    }
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let file_name = path.rsplit('/').next().unwrap_or_default();
    if let Some((_, extension)) = file_name.rsplit_once('.')
        && let Some(slug) = features::image_extension_feature(&extension.to_ascii_lowercase())
    {
        findings.record(slug, url);
    }
}

/**
 * Detect CSS features in `<style>` elements and `style` attributes.
 */
fn detect_styles(document: &Html, findings: &mut Findings) {
    static STYLE_ELEMENTS: OnceLock<Selector> = OnceLock::new();
    static STYLED_ELEMENTS: OnceLock<Selector> = OnceLock::new();
    for element in document.select(STYLE_ELEMENTS.get_or_init(|| static_selector("style"))) {
        let source: String = element.text().collect();
        record_css_comments(&source, findings);
        for rule in parse_stylesheet(&source) {
            analyse_rule(&rule, RuleContext::Document, document, findings);
        }
    }
    for element in document.select(STYLED_ELEMENTS.get_or_init(|| static_selector("[style]"))) {
        let source = element.value().attr("style").unwrap_or_default();
        record_css_comments(source, findings);
        for declaration in parse_declaration_list(source) {
            detect_declaration(&declaration, findings);
        }
    }
}

/**
 * Count the comments in CSS source by their opening delimiter.
 */
fn record_css_comments(source: &str, findings: &mut Findings) {
    for (index, _) in source.match_indices("/*") {
        let comment = source[index..]
            .split_inclusive("*/")
            .next()
            .unwrap_or_default();
        findings.record(features::CSS_COMMENTS, comment);
    }
}

/// Where a rule appears, which decides how its selectors are treated.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RuleContext {
    Document,
    /// Inside `@keyframes`, where preludes are keyframe offsets rather than selectors.
    Keyframes,
    /// Nested in a style rule, where selectors are relative to the parent.
    Nested,
}

/**
 * Analyse a rule and return whether it can apply to the document.
 *
 * Style rules whose selectors match no element are ignored entirely, so unused CSS (for example a
 * framework stylesheet) does not report features. At-rules count only when something inside them
 * applies, except for `@font-face`, `@import` and `@keyframes`, whose use cannot be traced.
 */
fn analyse_rule(
    rule: &CssRule,
    context: RuleContext,
    document: &Html,
    findings: &mut Findings,
) -> bool {
    match rule {
        CssRule::Style {
            selectors,
            declarations,
            nested_rules,
        } => {
            let usage = match context {
                RuleContext::Document => record_selector_features(selectors, document, findings),
                RuleContext::Keyframes | RuleContext::Nested => SelectorUsage {
                    applies: true,
                    interactive: false,
                },
            };
            if !usage.applies {
                return false;
            }
            findings.enter_graceful(usage.interactive);
            for declaration in declarations {
                detect_declaration(declaration, findings);
            }
            for nested_rule in nested_rules {
                findings.record_unverified(features::CSS_NESTING, &rule_example(nested_rule));
                analyse_rule(nested_rule, RuleContext::Nested, document, findings);
            }
            findings.leave_graceful(usage.interactive);
            true
        }
        CssRule::At {
            name,
            prelude,
            declarations,
            rules,
        } => {
            let example = format!("@{name} {prelude}");
            let unprefixed = unprefixed_at_rule_name(name);
            match unprefixed {
                "font-face" => {
                    findings.record(features::AT_FONT_FACE, &example);
                    return true;
                }
                "import" => {
                    findings.record(features::AT_IMPORT, &example);
                    return true;
                }
                "keyframes" => {
                    findings.record(features::AT_KEYFRAMES, &example);
                    findings.enter_graceful(true);
                    for keyframe in rules {
                        analyse_rule(keyframe, RuleContext::Keyframes, document, findings);
                    }
                    findings.leave_graceful(true);
                    return true;
                }
                _ => {}
            }
            // Clients that ignore a conditional block keep the unconditional styles.
            let conditional = matches!(unprefixed, "media" | "supports");
            findings.enter_graceful(conditional);
            let mut applies = !declarations.is_empty();
            for declaration in declarations {
                detect_declaration(declaration, findings);
            }
            for nested_rule in rules {
                applies |= analyse_rule(nested_rule, context, document, findings);
            }
            findings.leave_graceful(conditional);
            if applies {
                match unprefixed {
                    "media" => {
                        findings.record(features::AT_MEDIA, &example);
                        let media_features: BTreeSet<_> = analyse_value(prelude)
                            .identifiers
                            .iter()
                            .filter_map(|identifier| features::media_feature(identifier))
                            .collect();
                        for slug in media_features {
                            findings.record(slug, &example);
                        }
                    }
                    "supports" => findings.record(features::AT_SUPPORTS, &example),
                    _ => {}
                }
            }
            applies
        }
    }
}

/// Whether a selector list can apply to the document, and whether it applies only on interaction.
struct SelectorUsage {
    applies: bool,
    /// Every applicable selector depends on an interaction state such as `:hover`.
    interactive: bool,
}

/**
 * Record the selector features of the selectors in a list that can apply.
 *
 * Selectors that depend on an interaction state are graceful: clients without support skip the
 * rule and keep the resting style.
 */
fn record_selector_features(
    selectors: &str,
    document: &Html,
    findings: &mut Findings,
) -> SelectorUsage {
    let analysed = analyse_selector_list(selectors);
    let mut applies = false;
    let mut interactive = true;
    for selector in &analysed {
        let compiled = selector
            .static_selector
            .as_deref()
            .and_then(|source| Selector::parse(source).ok());
        let verified = match compiled {
            Some(compiled) if document.select(&compiled).next().is_none() => continue,
            Some(_) => true,
            None => false,
        };
        applies = true;
        let selector_interactive = selector
            .features
            .iter()
            .any(|feature| features::INTERACTION_PSEUDO_CLASSES.contains(feature));
        interactive &= selector_interactive;
        findings.enter_graceful(selector_interactive);
        for feature in &selector.features {
            if verified {
                findings.record(feature, &selector.text);
            } else {
                findings.record_unverified(feature, &selector.text);
            }
        }
        findings.leave_graceful(selector_interactive);
    }
    let interactive = applies && interactive;
    if applies && analysed.len() > 1 {
        findings.enter_graceful(interactive);
        findings.record(features::SELECTOR_GROUPING, selectors);
        findings.leave_graceful(interactive);
    }
    SelectorUsage {
        applies,
        interactive,
    }
}

/**
 * Detect the property, value, function, unit and image features of one declaration, counting each
 * feature once per declaration.
 */
fn detect_declaration(declaration: &Declaration, findings: &mut Findings) {
    let facts = analyse_value(&declaration.value);
    let mut found: BTreeSet<&'static str> = features::property_features(&declaration.name)
        .iter()
        .copied()
        .collect();
    if declaration.name.starts_with("--") {
        found.insert(features::CSS_VARIABLES);
    }
    found.extend(features::property_value_features(
        &declaration.name,
        &facts.identifiers,
    ));
    found.extend(
        facts
            .functions
            .iter()
            .filter_map(|function| features::function_feature(function)),
    );
    found.extend(
        facts
            .identifiers
            .iter()
            .filter_map(|identifier| features::keyword_feature(identifier)),
    );
    found.extend(
        facts
            .units
            .iter()
            .filter_map(|unit| features::unit_feature(unit)),
    );
    if facts.percentage {
        found.insert(features::UNIT_PERCENT);
    }
    if facts.important {
        found.insert(features::CSS_IMPORTANT);
    }
    let example = format!("{}: {}", declaration.name, declaration.value);
    for slug in found {
        findings.record(slug, &example);
    }
    for url in &facts.urls {
        record_image_url(url, findings);
    }
}

/**
 * Remove a vendor prefix from an at-rule name, as in `-webkit-keyframes`.
 */
fn unprefixed_at_rule_name(name: &str) -> &str {
    if name.starts_with('-') {
        name.splitn(3, '-').nth(2).unwrap_or(name)
    } else {
        name
    }
}

/**
 * Describe a rule by its prelude for use as an example.
 */
fn rule_example(rule: &CssRule) -> String {
    match rule {
        CssRule::Style { selectors, .. } => selectors.clone(),
        CssRule::At { name, prelude, .. } => format!("@{name} {prelude}"),
    }
}

/**
 * Render an element's start tag with its attributes, for use as an example.
 */
fn start_tag(element: ElementRef<'_>) -> String {
    let mut tag = format!("<{}", element.value().name());
    for (name, value) in element.value().attrs() {
        if value.is_empty() {
            tag.push_str(&format!(" {name}"));
        } else {
            tag.push_str(&format!(" {name}=\"{value}\""));
        }
        if tag.len() > EXAMPLE_LENGTH {
            break;
        }
    }
    tag.push('>');
    tag
}

/**
 * Collapse whitespace and truncate an example to [`EXAMPLE_LENGTH`] characters.
 */
fn shorten(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= EXAMPLE_LENGTH {
        return collapsed;
    }
    let mut shortened: String = collapsed.chars().take(EXAMPLE_LENGTH - 1).collect();
    shortened.push('…');
    shortened
}

/**
 * Report whether `haystack` contains `needle`, comparing ASCII letters case-insensitively.
 */
fn contains_ignoring_ascii_case(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}
