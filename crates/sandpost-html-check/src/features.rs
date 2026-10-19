//! Mapping from HTML, CSS and image constructs to caniemail feature slugs.
//!
//! Every slug used here must exist in the embedded data; the crate tests enforce it.

pub(crate) const SELECTOR_ADJACENT_SIBLING: &str = "css-selector-adjacent-sibling";
pub(crate) const SELECTOR_ATTRIBUTE: &str = "css-selector-attribute";
pub(crate) const SELECTOR_CHAINING: &str = "css-selector-chaining";
pub(crate) const SELECTOR_CHILD: &str = "css-selector-child";
pub(crate) const SELECTOR_CLASS: &str = "css-selector-class";
pub(crate) const SELECTOR_DESCENDANT: &str = "css-selector-descendant";
pub(crate) const SELECTOR_GENERAL_SIBLING: &str = "css-selector-general-sibling";
pub(crate) const SELECTOR_GROUPING: &str = "css-selector-grouping";
pub(crate) const SELECTOR_ID: &str = "css-selector-id";
pub(crate) const SELECTOR_TYPE: &str = "css-selector-type";
pub(crate) const SELECTOR_UNIVERSAL: &str = "css-selector-universal";

pub(crate) const AT_FONT_FACE: &str = "css-at-font-face";
pub(crate) const AT_IMPORT: &str = "css-at-import";
pub(crate) const AT_KEYFRAMES: &str = "css-at-keyframes";
pub(crate) const AT_MEDIA: &str = "css-at-media";
pub(crate) const AT_SUPPORTS: &str = "css-at-supports";

pub(crate) const CSS_COMMENTS: &str = "css-comments";
pub(crate) const CSS_IMPORTANT: &str = "css-important";
pub(crate) const CSS_NESTING: &str = "css-nesting";
pub(crate) const CSS_VARIABLES: &str = "css-variables";
pub(crate) const UNIT_PERCENT: &str = "css-unit-percent";

pub(crate) const HTML_AMP: &str = "amp";
pub(crate) const HTML_BODY: &str = "html-body";
pub(crate) const HTML_COMMENTS: &str = "html-comments";
pub(crate) const HTML_DOCTYPE: &str = "html-doctype";
pub(crate) const IMAGE_BASE64: &str = "image-base64";

/// Document-structure features that clients strip or ignore without breaking the message, such as
/// webmail removing `<body>` or the doctype. They are reported but excluded from client scores;
/// rules inside a removed `<style>` element are still scored through their own features.
pub(crate) const UNSCORED_FEATURES: &[&str] = &[
    HTML_BODY,
    HTML_DOCTYPE,
    HTML_COMMENTS,
    CSS_COMMENTS,
    "html-link",
    "html-style",
];

/// Features whose absence degrades gracefully: clients that do not support them only lose an
/// enhancement (dark mode, hover and responsive styles, motion), accessibility metadata or a
/// cosmetic detail, and the message stays readable. caniemail does not classify features, so this
/// list is maintained here.
pub(crate) const GRACEFUL_FEATURES: &[&str] = &[
    AT_MEDIA,
    AT_KEYFRAMES,
    "css-at-media-device-pixel-ratio",
    "css-at-media-hover",
    "css-at-media-orientation",
    "css-at-media-prefers-color-scheme",
    "css-at-media-prefers-reduced-motion",
    "css-color-scheme",
    "html-meta-color-scheme",
    "css-pseudo-class-active",
    "css-pseudo-class-focus",
    "css-pseudo-class-focus-visible",
    "css-pseudo-class-focus-within",
    "css-pseudo-class-hover",
    "css-pseudo-class-visited",
    "css-animation",
    "css-transition",
    "css-border-radius",
    "css-box-shadow",
    "css-text-shadow",
    "html-aria-describedby",
    "html-aria-hidden",
    "html-aria-label",
    "html-aria-labelledby",
    "html-aria-live",
    "html-dir",
    "html-lang",
    "html-role",
];

/// Pseudo-classes that apply only in an interaction state; rules using them are enhancements.
pub(crate) const INTERACTION_PSEUDO_CLASSES: &[&str] = &[
    "css-pseudo-class-active",
    "css-pseudo-class-checked",
    "css-pseudo-class-focus",
    "css-pseudo-class-focus-visible",
    "css-pseudo-class-focus-within",
    "css-pseudo-class-hover",
    "css-pseudo-class-target",
    "css-pseudo-class-visited",
];

/// Client families shown by default: the clients most recipients use.
pub(crate) const MAIN_CLIENT_FAMILIES: &[&str] =
    &["apple-mail", "gmail", "outlook", "yahoo", "samsung-email"];

/// HTML features detected by matching a selector against the parsed document.
pub(crate) const MARKUP_DETECTORS: &[(&str, &str)] = &[
    ("html-abbr", "abbr"),
    ("html-acronym", "acronym"),
    ("html-address", "address"),
    ("html-align", "[align]"),
    ("html-anchor-links", "a[href^='#']"),
    ("html-aria-describedby", "[aria-describedby]"),
    ("html-aria-hidden", "[aria-hidden]"),
    ("html-aria-label", "[aria-label]"),
    ("html-aria-labelledby", "[aria-labelledby]"),
    ("html-aria-live", "[aria-live]"),
    ("html-audio", "audio"),
    ("html-background", "[background]"),
    ("html-base", "base"),
    ("html-bdi", "bdi"),
    ("html-blockquote", "blockquote"),
    ("html-button-reset", "button[type='reset' i]"),
    (
        "html-button-submit",
        "button[type='submit' i], form button:not([type])",
    ),
    ("html-cellpadding", "[cellpadding]"),
    ("html-cellspacing", "[cellspacing]"),
    ("html-code", "code"),
    ("html-command-attribute", "[command], [commandfor]"),
    ("html-del", "del"),
    ("html-dfn", "dfn"),
    ("html-dialog", "dialog"),
    ("html-dir", "[dir]"),
    ("html-div", "div"),
    ("html-form", "form"),
    ("html-h1-h6", "h1, h2, h3, h4, h5, h6"),
    ("html-height", "[height]"),
    ("html-hidden", "[hidden]"),
    ("html-hr", "hr"),
    ("html-image-maps", "map, img[usemap]"),
    ("html-img", "img"),
    ("html-input-checkbox", "input[type='checkbox' i]"),
    ("html-input-hidden", "input[type='hidden' i]"),
    ("html-input-radio", "input[type='radio' i]"),
    ("html-input-reset", "input[type='reset' i]"),
    ("html-input-submit", "input[type='submit' i]"),
    ("html-input-text", "input[type='text' i], input:not([type])"),
    ("html-lang", "[lang]"),
    ("html-link", "link"),
    ("html-lists", "ul, ol, dl"),
    ("html-loading-attribute", "[loading]"),
    ("html-mailto-links", "a[href^='mailto:' i]"),
    ("html-marquee", "marquee"),
    (
        "html-meta-color-scheme",
        "meta[name='color-scheme' i], meta[name='supported-color-schemes' i]",
    ),
    ("html-meter", "meter"),
    ("html-object", "object"),
    ("html-p", "p"),
    ("html-picture", "picture"),
    ("html-popover", "[popover]"),
    ("html-pre", "pre"),
    ("html-progress", "progress"),
    ("html-required", "[required]"),
    ("html-role", "[role]"),
    ("html-rp", "rp"),
    ("html-rt", "rt"),
    ("html-ruby", "ruby"),
    ("html-select", "select"),
    (
        "html-semantics",
        "article, aside, details, figcaption, figure, footer, header, main, mark, nav, section, summary, time",
    ),
    ("html-small", "small"),
    ("html-span", "span"),
    ("html-srcset", "[srcset], [sizes]"),
    ("html-strike", "strike"),
    ("html-strong", "strong"),
    ("html-style", "style"),
    ("html-svg", "svg"),
    ("html-table", "table"),
    ("html-target", "[target]"),
    ("html-textarea", "textarea"),
    ("html-valign", "[valign]"),
    ("html-video", "video"),
    ("html-wbr", "wbr"),
    ("html-width", "[width]"),
    ("css-inert-attribute", "[inert]"),
];

/**
 * Return the features of a property name, ignoring vendor prefixes.
 */
pub(crate) fn property_features(name: &str) -> &'static [&'static str] {
    let name = ["-webkit-", "-moz-", "-ms-", "-o-"]
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))
        .unwrap_or(name);
    match name {
        "accent-color" => &["css-accent-color"],
        "align-items" => &["css-align-items"],
        "aspect-ratio" => &["css-aspect-ratio"],
        "backdrop-filter" => &["css-backdrop-filter"],
        "background" => &["css-background"],
        "background-blend-mode" => &["css-background-blend-mode"],
        "background-clip" => &["css-background-clip"],
        "background-color" => &["css-background-color"],
        "background-image" => &["css-background-image"],
        "background-origin" => &["css-background-origin"],
        "background-position" | "background-position-x" | "background-position-y" => {
            &["css-background-position"]
        }
        "background-repeat" => &["css-background-repeat"],
        "background-size" => &["css-background-size"],
        "block-size" => &["css-block-inline-size"],
        "inline-size" => &["css-block-inline-size", "css-inline-size"],
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left"
        | "border-width" | "border-style" | "border-color" => &["css-border"],
        "border-collapse" => &["css-border-collapse"],
        "border-spacing" => &["css-border-spacing"],
        "border-radius"
        | "border-top-left-radius"
        | "border-top-right-radius"
        | "border-bottom-left-radius"
        | "border-bottom-right-radius" => &["css-border-radius"],
        "border-start-start-radius"
        | "border-start-end-radius"
        | "border-end-start-radius"
        | "border-end-end-radius" => &["css-border-radius-logical"],
        "border-inline" | "border-block" => &["css-border-inline-block"],
        "border-inline-start" | "border-inline-end" | "border-block-start" | "border-block-end" => {
            &["css-border-inline-block-individual"]
        }
        "box-shadow" => &["css-box-shadow"],
        "box-sizing" => &["css-box-sizing"],
        "caption-side" => &["css-caption-side"],
        "clear" => &["css-clear"],
        "clip-path" => &["css-clip-path"],
        "color-scheme" => &["css-color-scheme"],
        "column-count" => &["css-column-count", "css-column-layout-properties"],
        "columns" | "column-width" | "column-rule" | "column-rule-color" | "column-rule-style"
        | "column-rule-width" | "column-span" | "column-fill" => &["css-column-layout-properties"],
        "gap" | "row-gap" | "column-gap" | "grid-gap" | "grid-row-gap" | "grid-column-gap" => {
            &["css-gap"]
        }
        "cursor" => &["css-cursor"],
        "direction" => &["css-direction"],
        "display" => &["css-display"],
        "empty-cells" => &["css-empty-cells"],
        "filter" => &["css-filter"],
        "float" => &["css-float"],
        "font" => &["css-font"],
        "font-kerning" => &["css-font-kerning"],
        "font-size" => &["css-font-size"],
        "font-size-adjust" => &["css-font-size-adjust"],
        "font-stretch" => &["css-font-stretch"],
        "font-weight" => &["css-font-weight"],
        "grid-template"
        | "grid-template-columns"
        | "grid-template-rows"
        | "grid-template-areas" => &["css-grid-template"],
        "height" => &["css-height"],
        "hyphenate-character" => &["css-hyphenate-character"],
        "hyphenate-limit-chars" => &["css-hyphenate-limit-chars"],
        "hyphens" => &["css-hyphens"],
        "inset" => &["css-inset"],
        "justify-content" => &["css-justify-content"],
        "left" | "right" | "top" | "bottom" => &["css-left-right-top-bottom"],
        "letter-spacing" => &["css-letter-spacing"],
        "line-height" => &["css-line-height"],
        "list-style" => &["css-list-style"],
        "list-style-image" => &["css-list-style-image"],
        "list-style-position" => &["css-list-style-position"],
        "list-style-type" => &["css-list-style-type"],
        "margin" | "margin-top" | "margin-right" | "margin-bottom" | "margin-left" => {
            &["css-margin"]
        }
        "margin-block-start" | "margin-block-end" => &["css-margin-block-start-end"],
        "margin-inline" | "margin-block" => &["css-margin-inline-block"],
        "margin-inline-start" | "margin-inline-end" => &["css-margin-inline-start-end"],
        "mask-image" => &["css-mask-image"],
        "max-block-size" => &["css-max-block-size"],
        "max-height" => &["css-max-height"],
        "max-inline-size" => &["css-max-inline-size"],
        "max-width" => &["css-max-width"],
        "min-block-size" => &["css-min-block-size"],
        "min-height" => &["css-min-height"],
        "min-inline-size" => &["css-min-inline-size"],
        "min-width" => &["css-min-width"],
        "mix-blend-mode" => &["css-mix-blend-mode"],
        "object-fit" => &["css-object-fit"],
        "object-position" => &["css-object-position"],
        "opacity" => &["css-opacity"],
        "orphans" => &["css-orphans"],
        "outline" | "outline-color" | "outline-style" | "outline-width" => &["css-outline"],
        "outline-offset" => &["css-outline-offset"],
        "overflow" | "overflow-x" | "overflow-y" => &["css-overflow"],
        "overflow-wrap" => &["css-overflow-wrap"],
        "padding" | "padding-top" | "padding-right" | "padding-bottom" | "padding-left" => {
            &["css-padding"]
        }
        "padding-block-start" | "padding-block-end" => &["css-padding-block-start-end"],
        "padding-inline" | "padding-block" => &["css-padding-inline-block"],
        "padding-inline-start" | "padding-inline-end" => &["css-padding-inline-start-end"],
        "position" => &["css-position"],
        "resize" => &["css-resize"],
        "shape-margin" => &["css-shape-margin"],
        "shape-outside" => &["css-shape-outside"],
        "tab-size" => &["css-tab-size"],
        "table-layout" => &["css-table-layout"],
        "text-align" => &["css-text-align"],
        "text-align-last" => &["css-text-align-last"],
        "text-decoration" => &["css-text-decoration"],
        "text-decoration-color" => &["css-text-decoration-color"],
        "text-decoration-line" => &["css-text-decoration-line"],
        "text-decoration-skip-ink" => &["css-text-decoration-skip-ink"],
        "text-decoration-style" => &["css-text-decoration-style"],
        "text-decoration-thickness" => &["css-text-decoration-thickness"],
        "text-emphasis" | "text-emphasis-color" | "text-emphasis-style" => &["css-text-emphasis"],
        "text-emphasis-position" => &["css-text-emphasis-position"],
        "text-indent" => &["css-text-indent"],
        "text-justify" => &["css-text-justify"],
        "text-orientation" => &["css-text-orientation"],
        "text-overflow" => &["css-text-overflow"],
        "text-shadow" => &["css-text-shadow"],
        "text-transform" => &["css-text-transform"],
        "text-underline-offset" => &["css-text-underline-offset"],
        "text-underline-position" => &["css-text-underline-position"],
        "text-wrap" | "text-wrap-mode" | "text-wrap-style" => &["css-text-wrap"],
        "transform" | "transform-origin" => &["css-transform"],
        "user-select" => &["css-user-select"],
        "vertical-align" => &["css-vertical-align"],
        "visibility" => &["css-visibility"],
        "white-space" => &["css-white-space"],
        "white-space-collapse" => &["css-white-space-collapse"],
        "widows" => &["css-widows"],
        "width" => &["css-width"],
        "word-break" => &["css-word-break"],
        "word-spacing" => &["css-word-spacing"],
        "word-wrap" => &["css-word-wrap"],
        "writing-mode" => &["css-writing-mode"],
        "z-index" => &["css-z-index"],
        _ => prefixed_property_features(name),
    }
}

/**
 * Return the features of property families recognized by their prefix.
 */
fn prefixed_property_features(name: &str) -> &'static [&'static str] {
    if name.starts_with("animation") {
        &["css-animation"]
    } else if name.starts_with("transition") {
        &["css-transition"]
    } else if name.starts_with("scroll-snap") {
        &["css-scroll-snap"]
    } else if name.starts_with("border-image") {
        &["css-border-image"]
    } else if (name.starts_with("border-inline-") || name.starts_with("border-block-"))
        && (name.ends_with("-width") || name.ends_with("-style") || name.ends_with("-color"))
    {
        &["css-border-inline-block-longhand"]
    } else {
        &[]
    }
}

/**
 * Return the features of a value that depend on the property, such as `display: grid`.
 */
pub(crate) fn property_value_features(name: &str, identifiers: &[String]) -> Vec<&'static str> {
    let has = |candidates: &[&str]| {
        identifiers
            .iter()
            .any(|identifier| candidates.contains(&identifier.as_str()))
    };
    let mut found = Vec::new();
    match name {
        "display" => {
            if has(&["flex", "inline-flex", "-webkit-flex", "-webkit-box"]) {
                found.push("css-display-flex");
            }
            if has(&["grid", "inline-grid"]) {
                found.push("css-display-grid");
            }
            if has(&["none"]) {
                found.push("css-display-none");
            }
        }
        "flex-direction" if has(&["column", "column-reverse"]) => found.push("css-flex-direction"),
        "flex-wrap" if has(&["wrap", "wrap-reverse"]) => found.push("css-flex-wrap"),
        "flex-flow" => {
            if has(&["column", "column-reverse"]) {
                found.push("css-flex-direction");
            }
            if has(&["wrap", "wrap-reverse"]) {
                found.push("css-flex-wrap");
            }
        }
        _ => {}
    }
    found
}

/**
 * Return the feature of a CSS function used in a value.
 */
pub(crate) fn function_feature(name: &str) -> Option<&'static str> {
    let name = name.strip_prefix("-webkit-").unwrap_or(name);
    match name {
        "calc" => Some("css-unit-calc"),
        "clamp" => Some("css-function-clamp"),
        "conic-gradient" | "repeating-conic-gradient" => Some("css-conic-gradient"),
        "light-dark" => Some("css-function-light-dark"),
        "linear-gradient" | "repeating-linear-gradient" => Some("css-linear-gradient"),
        "max" => Some("css-function-max"),
        "min" => Some("css-function-min"),
        "radial-gradient" | "repeating-radial-gradient" => Some("css-radial-gradient"),
        "rgb" => Some("css-rgb"),
        "rgba" => Some("css-rgba"),
        "lch" | "oklch" | "lab" | "oklab" => Some("css-modern-color"),
        "var" => Some(CSS_VARIABLES),
        _ => None,
    }
}

/**
 * Return the feature of a keyword used in a value.
 */
pub(crate) fn keyword_feature(identifier: &str) -> Option<&'static str> {
    match identifier {
        "fit-content" | "min-content" | "max-content" | "-webkit-fit-content" => {
            Some("css-intrinsic-size")
        }
        "system-ui" | "ui-serif" | "ui-sans-serif" | "ui-rounded" | "ui-monospace" => {
            Some("css-sytem-ui")
        }
        "initial" => Some("css-unit-initial"),
        _ => None,
    }
}

/**
 * Return the feature of a length unit.
 */
pub(crate) fn unit_feature(unit: &str) -> Option<&'static str> {
    match unit {
        "ch" => Some("css-unit-ch"),
        "cm" => Some("css-unit-cm"),
        "em" => Some("css-unit-em"),
        "ex" => Some("css-unit-ex"),
        "in" => Some("css-unit-in"),
        "mm" => Some("css-unit-mm"),
        "pc" => Some("css-unit-pc"),
        "pt" => Some("css-unit-pt"),
        "px" => Some("css-unit-px"),
        "rem" => Some("css-unit-rem"),
        "vh" => Some("css-unit-vh"),
        "vmax" => Some("css-unit-vmax"),
        "vmin" => Some("css-unit-vmin"),
        "vw" => Some("css-unit-vw"),
        _ => None,
    }
}

/**
 * Return the feature of a media feature name such as `-webkit-min-device-pixel-ratio`.
 */
pub(crate) fn media_feature(identifier: &str) -> Option<&'static str> {
    let identifier = identifier.strip_prefix("-webkit-").unwrap_or(identifier);
    let identifier = identifier
        .strip_prefix("min-")
        .or_else(|| identifier.strip_prefix("max-"))
        .unwrap_or(identifier);
    match identifier {
        "device-pixel-ratio" => Some("css-at-media-device-pixel-ratio"),
        "hover" | "any-hover" => Some("css-at-media-hover"),
        "orientation" => Some("css-at-media-orientation"),
        "prefers-color-scheme" => Some("css-at-media-prefers-color-scheme"),
        "prefers-reduced-motion" => Some("css-at-media-prefers-reduced-motion"),
        _ => None,
    }
}

/**
 * Return the feature of a pseudo-class name.
 */
pub(crate) fn pseudo_class_feature(name: &str) -> Option<&'static str> {
    match name {
        "active" => Some("css-pseudo-class-active"),
        "checked" => Some("css-pseudo-class-checked"),
        "default" => Some("css-pseudo-class-default"),
        "first-child" => Some("css-pseudo-class-first-child"),
        "first-of-type" => Some("css-pseudo-class-first-of-type"),
        "focus" => Some("css-pseudo-class-focus"),
        "focus-visible" => Some("css-pseudo-class-focus-visible"),
        "focus-within" => Some("css-pseudo-class-focus-within"),
        "has" => Some("css-pseudo-class-has"),
        "hover" => Some("css-pseudo-class-hover"),
        "lang" => Some("css-pseudo-class-lang"),
        "last-child" => Some("css-pseudo-class-last-child"),
        "last-of-type" => Some("css-pseudo-class-last-of-type"),
        "link" => Some("css-pseudo-class-link"),
        "not" => Some("css-pseudo-class-not"),
        "nth-child" => Some("css-pseudo-class-nth-child"),
        "nth-last-child" => Some("css-pseudo-class-nth-last-child"),
        "nth-last-of-type" => Some("css-pseudo-class-nth-last-of-type"),
        "nth-of-type" => Some("css-pseudo-class-nth-of-type"),
        "only-child" => Some("css-pseudo-class-only-child"),
        "only-of-type" => Some("css-pseudo-class-only-of-type"),
        "target" => Some("css-pseudo-class-target"),
        "visited" => Some("css-pseudo-class-visited"),
        _ => None,
    }
}

/**
 * Return the feature of a pseudo-element name.
 */
pub(crate) fn pseudo_element_feature(name: &str) -> Option<&'static str> {
    match name {
        "after" => Some("css-pseudo-element-after"),
        "before" => Some("css-pseudo-element-before"),
        "first-letter" => Some("css-pseudo-element-first-letter"),
        "first-line" => Some("css-pseudo-element-first-line"),
        "marker" => Some("css-pseudo-element-marker"),
        "placeholder" => Some("css-pseudo-element-placeholder"),
        _ => None,
    }
}

/**
 * Return the image format feature of a file extension.
 */
pub(crate) fn image_extension_feature(extension: &str) -> Option<&'static str> {
    match extension {
        "apng" => Some("image-apng"),
        "avif" => Some("image-avif"),
        "bmp" => Some("image-bmp"),
        "gif" => Some("image-gif"),
        "hdr" => Some("image-hdr"),
        "heif" | "heic" => Some("image-heif"),
        "ico" => Some("image-ico"),
        "jpg" | "jpeg" | "jfif" => Some("image-jpg"),
        "mp4" => Some("image-mp4"),
        "png" => Some("image-png"),
        "svg" => Some("image-svg"),
        "tif" | "tiff" => Some("image-tiff"),
        "webp" => Some("image-webp"),
        _ => None,
    }
}

/**
 * Return the image format feature of a media type such as `image/webp`.
 */
pub(crate) fn image_media_type_feature(media_type: &str) -> Option<&'static str> {
    match media_type {
        "image/apng" => Some("image-apng"),
        "image/avif" => Some("image-avif"),
        "image/bmp" => Some("image-bmp"),
        "image/gif" => Some("image-gif"),
        "image/vnd.radiance" => Some("image-hdr"),
        "image/heif" | "image/heic" => Some("image-heif"),
        "image/x-icon" | "image/vnd.microsoft.icon" => Some("image-ico"),
        "image/jpeg" | "image/jpg" => Some("image-jpg"),
        "video/mp4" => Some("image-mp4"),
        "image/png" => Some("image-png"),
        "image/svg+xml" => Some("image-svg"),
        "image/tiff" => Some("image-tiff"),
        "image/webp" => Some("image-webp"),
        _ => None,
    }
}
