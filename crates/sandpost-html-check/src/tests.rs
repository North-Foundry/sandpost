//! Tests for detection, data integrity and scoring.

use std::collections::BTreeSet;

use crate::database::{SupportLevel, parse_support, support_database};
use crate::detection::{Findings, detect};
use crate::features::{self, MARKUP_DETECTORS};
use crate::selector::analyse_selector_list;
use crate::stylesheet::{CssRule, analyse_value, parse_declaration_list, parse_stylesheet};
use crate::{HTML_SIZE_LIMIT, HtmlCheckError, check_html};

/**
 * Detect the features of an HTML document.
 */
fn findings(html: &str) -> Findings {
    detect(html).findings
}

/**
 * Return the number of occurrences of a feature, zero when it was not detected.
 */
fn occurrences(findings: &Findings, slug: &str) -> usize {
    findings.get(slug).map_or(0, |finding| finding.occurrences)
}

/**
 * Collect every slug referenced by the detector tables and mapping functions.
 */
fn referenced_slugs() -> BTreeSet<&'static str> {
    let mut slugs: BTreeSet<&'static str> =
        MARKUP_DETECTORS.iter().map(|(slug, _)| *slug).collect();
    slugs.extend([
        features::SELECTOR_ADJACENT_SIBLING,
        features::SELECTOR_ATTRIBUTE,
        features::SELECTOR_CHAINING,
        features::SELECTOR_CHILD,
        features::SELECTOR_CLASS,
        features::SELECTOR_DESCENDANT,
        features::SELECTOR_GENERAL_SIBLING,
        features::SELECTOR_GROUPING,
        features::SELECTOR_ID,
        features::SELECTOR_TYPE,
        features::SELECTOR_UNIVERSAL,
        features::AT_FONT_FACE,
        features::AT_IMPORT,
        features::AT_KEYFRAMES,
        features::AT_MEDIA,
        features::AT_SUPPORTS,
        features::CSS_COMMENTS,
        features::CSS_IMPORTANT,
        features::CSS_NESTING,
        features::CSS_VARIABLES,
        features::UNIT_PERCENT,
        features::HTML_AMP,
        features::HTML_BODY,
        features::HTML_COMMENTS,
        features::HTML_DOCTYPE,
        features::IMAGE_BASE64,
    ]);
    slugs.extend(features::UNSCORED_FEATURES);
    slugs.extend(features::GRACEFUL_FEATURES);
    for feature in support_database().data.iter() {
        let name = feature.slug.trim_start_matches("css-");
        slugs.extend(features::property_features(name));
        slugs.extend(features::function_feature(name));
        slugs.extend(features::keyword_feature(name));
        slugs.extend(features::media_feature(name));
        slugs.extend(features::pseudo_class_feature(
            name.trim_start_matches("pseudo-class-"),
        ));
        slugs.extend(features::pseudo_element_feature(
            name.trim_start_matches("pseudo-element-"),
        ));
        slugs.extend(features::unit_feature(name.trim_start_matches("unit-")));
        slugs.extend(features::image_extension_feature(
            name.trim_start_matches("image-"),
        ));
    }
    for property in ["display", "flex-direction", "flex-wrap", "flex-flow"] {
        let identifiers = ["flex", "grid", "none", "column", "wrap"].map(str::to_owned);
        slugs.extend(features::property_value_features(property, &identifiers));
    }
    for name in [
        "-webkit-calc",
        "repeating-conic-gradient",
        "repeating-linear-gradient",
        "repeating-radial-gradient",
        "oklch",
        "animation-name",
        "transition-duration",
        "scroll-snap-type",
        "border-image-source",
        "border-inline-start-width",
        "border-start-start-radius",
        "inline-size",
        "column-count",
        "grid-template-columns",
        "text-wrap-mode",
    ] {
        slugs.extend(features::function_feature(name));
        slugs.extend(features::property_features(name));
    }
    for media_type in [
        "image/webp",
        "image/svg+xml",
        "image/vnd.radiance",
        "video/mp4",
        "image/x-icon",
    ] {
        slugs.extend(features::image_media_type_feature(media_type));
    }
    slugs
}

/**
 * The embedded snapshot deserializes and contains the expected categories.
 */
#[test]
fn embedded_data_loads() {
    let database = support_database();
    assert!(database.data.len() > 250);
    assert!(
        database
            .data
            .iter()
            .any(|feature| feature.category == "css")
    );
    assert!(!database.last_update_date.is_empty());
    assert!(database.nicenames.family.contains_key("gmail"));
}

/**
 * Every slug the detectors can report exists in the data, so a lookup never fails silently.
 */
#[test]
fn every_referenced_slug_exists_in_the_data() {
    let database = support_database();
    let missing: Vec<_> = referenced_slugs()
        .into_iter()
        .filter(|slug| database.feature(slug).is_none())
        .collect();
    assert!(
        missing.is_empty(),
        "slugs missing from caniemail data: {missing:?}"
    );
}

/**
 * Every markup detector selector compiles.
 */
#[test]
fn every_markup_detector_compiles() {
    for (slug, selector) in MARKUP_DETECTORS {
        assert!(
            scraper::Selector::parse(selector).is_ok(),
            "{slug}: {selector}"
        );
    }
}

/**
 * Support codes keep supported-with-note as supported and unknown as unknown.
 */
#[test]
fn support_codes_parse_strictly() {
    assert_eq!(parse_support("y"), (SupportLevel::Yes, vec![]));
    assert_eq!(parse_support("y #2"), (SupportLevel::Yes, vec![2]));
    assert_eq!(
        parse_support("a #1 #3"),
        (SupportLevel::Partial, vec![1, 3])
    );
    assert_eq!(parse_support("n"), (SupportLevel::No, vec![]));
    assert_eq!(parse_support("u"), (SupportLevel::Unknown, vec![]));
    assert_eq!(parse_support(""), (SupportLevel::Unknown, vec![]));
}

/**
 * Stylesheets keep unknown properties, at-rules and nested rules.
 */
#[test]
fn stylesheets_parse_tolerantly() {
    let rules = parse_stylesheet(
        "<!-- @import url(a.css); .a { mso-line-height-rule: exactly; color: red !important; &:hover { color: blue } } \
         @media (max-width: 600px) { .b { width: 100% } } -->",
    );
    assert!(matches!(&rules[0], CssRule::At { name, .. } if name == "import"));
    let CssRule::Style {
        declarations,
        nested_rules,
        ..
    } = &rules[1]
    else {
        panic!("expected a style rule: {rules:?}");
    };
    assert_eq!(declarations[0].name, "mso-line-height-rule");
    assert_eq!(declarations[1].value, "red !important");
    assert_eq!(nested_rules.len(), 1);
    assert!(
        matches!(&rules[2], CssRule::At { name, rules, .. } if name == "media" && rules.len() == 1)
    );
}

/**
 * Style attributes parse into declarations, keeping custom property case.
 */
#[test]
fn declaration_lists_parse() {
    let declarations = parse_declaration_list("Display: FLEX; --Brand-Color: #f00; broken");
    assert_eq!(declarations.len(), 2);
    assert_eq!(declarations[0].name, "display");
    assert_eq!(declarations[1].name, "--Brand-Color");
}

/**
 * Value analysis finds nested functions, units, URLs and the important marker.
 */
#[test]
fn values_expose_their_tokens() {
    let facts = analyse_value("calc(100% - var(--gap, 2rem)) url('a.webp') url(b.png) !important");
    // An unquoted `url(...)` is a single URL token rather than a function.
    assert_eq!(facts.functions, ["calc", "var", "url"]);
    assert_eq!(facts.units, ["rem"]);
    assert!(facts.percentage);
    assert!(facts.important);
    assert_eq!(facts.urls, ["a.webp", "b.png"]);
}

/**
 * Selector analysis reports combinators and strips dynamic pseudo-classes from the static form.
 */
#[test]
fn selectors_report_features_and_static_forms() {
    let selectors = analyse_selector_list(
        "a.button:hover > span::before, #main ~ p:first-child, td[align] *, :focus",
    );
    assert_eq!(selectors.len(), 4);
    let first = &selectors[0];
    assert_eq!(first.static_selector.as_deref(), Some("a.button > span"));
    for feature in [
        features::SELECTOR_TYPE,
        features::SELECTOR_CLASS,
        features::SELECTOR_CHAINING,
        features::SELECTOR_CHILD,
        "css-pseudo-class-hover",
        "css-pseudo-element-before",
    ] {
        assert!(
            first.features.contains(feature),
            "{feature} in {:?}",
            first.features
        );
    }
    assert_eq!(
        selectors[1].static_selector.as_deref(),
        Some("#main ~ p:first-child")
    );
    assert!(
        selectors[1]
            .features
            .contains(features::SELECTOR_GENERAL_SIBLING)
    );
    assert!(
        selectors[2]
            .features
            .contains(features::SELECTOR_DESCENDANT)
    );
    assert!(selectors[2].features.contains(features::SELECTOR_ATTRIBUTE));
    assert!(selectors[2].features.contains(features::SELECTOR_UNIVERSAL));
    assert_eq!(selectors[3].static_selector.as_deref(), Some("*"));
}

/**
 * Nesting selectors cannot be checked against the document.
 */
#[test]
fn nesting_selectors_are_unverifiable() {
    let selectors = analyse_selector_list("& > a");
    assert_eq!(selectors[0].static_selector, None);
}

/**
 * Markup detectors count elements and attributes, and the explicit body and doctype.
 */
#[test]
fn markup_features_are_detected() {
    let found = findings(
        "<!DOCTYPE html><html lang=\"en\"><body><!-- note --><table width=\"600\" cellpadding=\"0\"><tr><td valign=\"top\">\
         <video src=\"a.mp4\"></video><a href=\"#top\">Top</a><a href=\"mailto:a@example.test\">Mail</a>\
         <input type=\"Checkbox\"></td></tr></table></body></html>",
    );
    for slug in [
        "html-doctype",
        "html-lang",
        "html-body",
        "html-comments",
        "html-table",
        "html-width",
        "html-cellpadding",
        "html-valign",
        "html-video",
        "html-anchor-links",
        "html-mailto-links",
        "html-input-checkbox",
    ] {
        assert_eq!(occurrences(&found, slug), 1, "{slug}");
    }
    assert_eq!(
        found.get("html-video").unwrap().examples,
        ["<video src=\"a.mp4\">"]
    );
}

/**
 * A document without an explicit `<body>` tag does not report the body feature.
 */
#[test]
fn implicit_body_is_not_reported() {
    assert_eq!(occurrences(&findings("<p>Hello</p>"), "html-body"), 0);
}

/**
 * Image formats are recognized despite query strings, in srcset, picture sources, data URLs and CSS.
 */
#[test]
fn image_formats_are_detected() {
    let found = findings(
        "<img src=\"https://cdn.example.test/hero.WEBP?v=2#x\"><img srcset=\"a.avif 1x, b.png 2x\">\
         <picture><source type=\"image/svg+xml\" srcset=\"logo\"></picture>\
         <img src=\"data:image/gif;base64,R0lGOD,lh\"><div style=\"background-image: url('bg.jpg')\"></div>",
    );
    for slug in [
        "image-webp",
        "image-avif",
        "image-png",
        "image-svg",
        "image-base64",
        "image-gif",
        "image-jpg",
    ] {
        assert_eq!(occurrences(&found, slug), 1, "{slug}");
    }
}

/**
 * Inline styles report properties, values, functions, units and keywords once per declaration.
 */
#[test]
fn inline_style_features_are_detected() {
    let found = findings(
        "<div style=\"display : grid; padding: 1px 2px 3px; color: rgba(0,0,0,.5) !important; width: clamp(1rem, 50%, 30ch); \
         font-family: system-ui; --brand: red; height: fit-content\"></div>",
    );
    for slug in [
        "css-display",
        "css-display-grid",
        "css-padding",
        "css-rgba",
        "css-important",
        "css-function-clamp",
        "css-unit-rem",
        "css-unit-percent",
        "css-unit-ch",
        "css-sytem-ui",
        "css-variables",
        "css-intrinsic-size",
    ] {
        assert_eq!(occurrences(&found, slug), 1, "{slug}");
    }
    assert_eq!(occurrences(&found, "css-unit-px"), 1);
    assert_eq!(
        found.get("css-display-grid").unwrap().examples,
        ["display: grid"]
    );
}

/**
 * Rules whose selectors match nothing report neither selector nor declaration features.
 */
#[test]
fn unused_rules_are_ignored() {
    let found = findings(
        "<style>.unused:hover { display: flex } a:hover { text-shadow: 0 0 1px red } \
         @media (prefers-color-scheme: dark) { .missing { color: white } }</style><a href=\"/\">Link</a>",
    );
    assert_eq!(occurrences(&found, "css-display-flex"), 0);
    assert_eq!(occurrences(&found, "css-pseudo-class-hover"), 1);
    assert_eq!(
        found.get("css-pseudo-class-hover").unwrap().examples,
        ["a:hover"]
    );
    assert_eq!(occurrences(&found, "css-text-shadow"), 1);
    assert_eq!(occurrences(&found, "css-at-media"), 0);
    assert_eq!(occurrences(&found, "css-at-media-prefers-color-scheme"), 0);
}

/**
 * Used at-rules report their media features; font, import and keyframes rules always count.
 */
#[test]
fn at_rule_features_are_detected() {
    let found = findings(
        "<style>/* dark */ @import url(x.css); @font-face { font-family: X; src: url(x.woff2) } \
         @keyframes spin { from { transform: rotate(0) } to { transform: rotate(1turn) } } \
         @media (-webkit-min-device-pixel-ratio: 2) and (prefers-color-scheme: dark) { p { color: white } } \
         @supports (display: grid) { p { display: grid } }</style><p>Hi</p>",
    );
    for slug in [
        "css-comments",
        "css-at-import",
        "css-at-font-face",
        "css-at-keyframes",
        "css-at-media",
        "css-at-media-device-pixel-ratio",
        "css-at-media-prefers-color-scheme",
        "css-at-supports",
        "css-display-grid",
    ] {
        assert_eq!(occurrences(&found, slug), 1, "{slug}");
    }
    assert_eq!(occurrences(&found, "css-transform"), 2);
    assert_eq!(occurrences(&found, features::SELECTOR_TYPE), 2);
}

/**
 * Nested rules count as nesting with unverified selectors.
 */
#[test]
fn nested_rules_are_unverified() {
    let found = findings("<style>p { color: red; & span { opacity: .5 } }</style><p>Hi</p>");
    let nesting = found.get(features::CSS_NESTING).unwrap();
    assert_eq!(nesting.occurrences, 1);
    assert_eq!(nesting.unverified_occurrences, 1);
    assert_eq!(occurrences(&found, "css-opacity"), 1);
}

/**
 * The report scores each client by its latest tested version and sorts the worst features first.
 */
#[test]
fn report_scores_clients() {
    let report =
        check_html("<div style=\"display: grid\"><p style=\"color: red\">Hi</p></div>").unwrap();
    assert!(report.html_present);
    let summary = &report.summary;
    assert_eq!(summary.clients, report.clients.len());
    assert_eq!(
        summary.identical + summary.with_differences + summary.with_problems + summary.untested,
        summary.clients
    );
    assert!(
        summary.with_problems > 0,
        "display:grid breaks layout somewhere: {summary:?}"
    );
    let first = &report.features[0];
    let grid = report
        .features
        .iter()
        .position(|feature| feature.slug == "css-display-grid")
        .unwrap();
    let div = report
        .features
        .iter()
        .position(|feature| feature.slug == "html-div")
        .unwrap();
    assert!(grid < div);
    assert!(first.support.iter().any(|result| result.latest));
    let outlook = report
        .clients
        .iter()
        .find(|client| client.id == "outlook.windows")
        .unwrap();
    assert_eq!(outlook.family_name, "Outlook");
    assert_eq!(outlook.platform_name, "Windows");
    assert_eq!(report.data.license, "MIT");
}

/**
 * Only the last version of each client and feature is marked latest.
 */
#[test]
fn latest_versions_are_marked_once_per_client() {
    let report = check_html("<div style=\"display: flex\"></div>").unwrap();
    let flex = report
        .features
        .iter()
        .find(|feature| feature.slug == "css-display-flex")
        .unwrap();
    let mut latest_clients = BTreeSet::new();
    for result in flex.support.iter().filter(|result| result.latest) {
        assert!(
            latest_clients.insert(result.client.clone()),
            "{} marked latest twice",
            result.client
        );
    }
    let gmail_android: Vec<_> = flex
        .support
        .iter()
        .filter(|result| result.client == "gmail.android")
        .collect();
    assert!(gmail_android.last().unwrap().latest);
    assert!(
        gmail_android
            .iter()
            .rev()
            .skip(1)
            .all(|result| !result.latest)
    );
}

/**
 * Bodies above the size limit are refused.
 */
#[test]
fn oversized_bodies_are_refused() {
    let html = "a".repeat(HTML_SIZE_LIMIT + 1);
    assert_eq!(
        check_html(&html),
        Err(HtmlCheckError::TooLarge {
            size: HTML_SIZE_LIMIT + 1,
            limit: HTML_SIZE_LIMIT
        })
    );
}

/**
 * Linked stylesheets are counted and never fetched.
 */
#[test]
fn external_stylesheets_are_counted() {
    let report = check_html(
        "<link rel=\"Stylesheet preload\" href=\"https://example.test/a.css\"><p>Hi</p>",
    )
    .unwrap();
    assert_eq!(report.external_stylesheets, 1);
}

/**
 * Deeply nested CSS does not overflow the stack.
 */
#[test]
fn deep_nesting_is_bounded() {
    let value = format!("width: {}1px{}", "calc(".repeat(5000), ")".repeat(5000));
    let rules = format!(
        "{}p {{ color: red }}{}",
        "@media screen {".repeat(5000),
        "}".repeat(5000)
    );
    let html = format!("<style>{rules}</style><p style=\"{value}\">Hi</p>");
    assert!(check_html(&html).is_ok());
}

/**
 * Structural features that clients strip are listed but do not fail clients.
 */
#[test]
fn unscored_features_do_not_affect_client_outcomes() {
    let report = check_html("<!DOCTYPE html><html><head><style>p { color: red }</style></head><body><p>Hi</p></body></html>")
        .unwrap();
    for slug in ["html-body", "html-doctype", "html-style"] {
        let feature = report
            .features
            .iter()
            .find(|feature| feature.slug == slug)
            .unwrap();
        assert!(!feature.scored, "{slug} is not scored");
    }
    let body = report
        .features
        .iter()
        .find(|feature| feature.slug == "html-body")
        .unwrap();
    assert!(
        body.support
            .iter()
            .any(|result| result.latest && result.support == crate::SupportLevel::No)
    );
    let first_unscored = report
        .features
        .iter()
        .position(|feature| !feature.scored)
        .unwrap();
    assert!(
        report.features[first_unscored..]
            .iter()
            .all(|feature| !feature.scored),
        "scored features come first"
    );
    assert!(
        report.summary.with_problems < report.summary.clients / 4,
        "stripped structure must not fail most clients: {:?}",
        report.summary
    );
}

/**
 * The weighted score takes the worst feature weighted by its share of body elements.
 */
#[test]
fn weighted_score_uses_the_worst_weighted_feature() {
    let rare = check_html(&format!(
        "<body><div style=\"display: grid\"></div>{}</body>",
        "<p>x</p>".repeat(19)
    ))
    .unwrap();
    let common = check_html("<body><div style=\"display: grid\"></div></body>").unwrap();
    assert_eq!(rare.element_count, 20);
    assert_eq!(common.element_count, 1);
    let grid = |report: &crate::CompatibilityReport| {
        report
            .features
            .iter()
            .find(|feature| feature.slug == "css-display-grid")
            .unwrap()
            .impact
    };
    assert!(
        (grid(&common) / grid(&rare) - 20.0).abs() < 1.0,
        "impact scales with element share"
    );
    assert!(rare.weighted.supported > common.weighted.supported);
    let total = common.weighted.supported + common.weighted.partial + common.weighted.unsupported;
    assert!((total - 100.0).abs() < 0.2, "{:?}", common.weighted);
}

/**
 * Clients that only miss enhancements, accessibility metadata or cosmetic details render with
 * differences, never with problems.
 */
#[test]
fn graceful_features_cause_differences_not_problems() {
    let report = check_html(
        "<html lang=\"en\"><head><meta name=\"color-scheme\" content=\"light dark\"><style>\
         a:hover { color: red } @media (prefers-color-scheme: dark) { a { color: white } }</style></head>\
         <body><table role=\"presentation\"><tr><td><a href=\"/\" style=\"border-radius: 4px\">Go</a></td></tr></table></body></html>",
    )
    .unwrap();
    for slug in [
        "css-pseudo-class-hover",
        "css-at-media-prefers-color-scheme",
        "html-meta-color-scheme",
        "css-border-radius",
        "html-role",
    ] {
        let feature = report
            .features
            .iter()
            .find(|feature| feature.slug == slug)
            .unwrap();
        assert!(feature.graceful, "{slug} is graceful");
    }
    assert_eq!(report.summary.with_problems, 0, "{:?}", report.summary);
    assert!(report.summary.with_differences > 0, "{:?}", report.summary);
}

/**
 * A breaking feature makes clients without support fail, and is listed before graceful ones.
 */
#[test]
fn breaking_features_come_before_graceful_ones() {
    let report = check_html(
        "<body><div style=\"display: grid; border-radius: 4px\"><a href=\"/\">Go</a></div></body>",
    )
    .unwrap();
    let grid = report
        .features
        .iter()
        .position(|feature| feature.slug == "css-display-grid")
        .unwrap();
    let radius = report
        .features
        .iter()
        .position(|feature| feature.slug == "css-border-radius")
        .unwrap();
    assert!(grid < radius);
    assert!(report.summary.with_problems > 0);
}

/**
 * Main clients are the popular families; regional webmail is not.
 */
#[test]
fn main_clients_are_flagged() {
    let report = check_html("<p>Hi</p>").unwrap();
    let main = |id: &str| {
        report
            .clients
            .iter()
            .find(|client| client.id == id)
            .unwrap()
            .main
    };
    assert!(main("gmail.android"));
    assert!(main("outlook.windows"));
    assert!(main("apple-mail.ios"));
    assert!(!main("sfr.desktop-webmail"));
    assert!(!main("gmx.desktop-webmail"));
}

/**
 * A feature used only inside conditional or interaction rules is graceful; one also used
 * unconditionally is not.
 */
#[test]
fn features_in_graceful_contexts_are_graceful() {
    let report = check_html(
        "<style>@media (max-width: 600px) { p { max-height: 10px } } a:hover { text-decoration: underline }</style>\
         <p style=\"max-width: 10px\">Hi <a href=\"/\">x</a></p>",
    )
    .unwrap();
    let graceful = |slug: &str| {
        report
            .features
            .iter()
            .find(|feature| feature.slug == slug)
            .unwrap()
            .graceful
    };
    assert!(graceful("css-max-height"), "only inside @media");
    assert!(graceful("css-text-decoration"), "only inside :hover");
    assert!(!graceful("css-max-width"), "used inline");
    assert!(!graceful("css-unit-px"), "also used inline");
}
