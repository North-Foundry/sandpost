//! Selector feature analysis for `<style>` rules.
//!
//! Each selector of a list yields the caniemail selector features it uses and, when possible, a
//! static equivalent without user-interaction pseudo-classes and pseudo-elements. Matching the
//! static selector against the document tells whether the rule can apply to anything at all.

use std::collections::BTreeSet;

use cssparser::{ParseError, Parser, ParserInput, SourcePosition, Token, serialize_identifier};

use crate::features;

/// One selector of a selector list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnalysedSelector {
    pub(crate) text: String,
    pub(crate) features: BTreeSet<&'static str>,
    /// The selector without dynamic pseudo-classes and pseudo-elements, or `None` when it uses
    /// syntax that cannot be checked against the document (such as nesting `&`).
    pub(crate) static_selector: Option<String>,
}

/// Pseudo-classes that depend only on the document tree, so they stay in the static selector.
const STRUCTURAL_PSEUDO_CLASSES: &[&str] = &[
    "first-child",
    "last-child",
    "first-of-type",
    "last-of-type",
    "nth-child",
    "nth-last-child",
    "nth-of-type",
    "nth-last-of-type",
    "only-child",
    "only-of-type",
    "not",
    "has",
    "is",
    "where",
    "root",
    "empty",
];

/// Pseudo-elements that older CSS also allows with a single colon.
const LEGACY_PSEUDO_ELEMENTS: &[&str] = &["before", "after", "first-letter", "first-line"];

/// Incremental state for the selector being read.
struct SelectorBuilder {
    features: BTreeSet<&'static str>,
    static_text: String,
    verifiable: bool,
    in_compound: bool,
    compound_simple_selectors: usize,
    compound_has_static_part: bool,
    pending_whitespace: bool,
    after_combinator: bool,
}

impl SelectorBuilder {
    /**
     * Start an empty selector.
     */
    fn new() -> Self {
        Self {
            features: BTreeSet::new(),
            static_text: String::new(),
            verifiable: true,
            in_compound: false,
            compound_simple_selectors: 0,
            compound_has_static_part: false,
            pending_whitespace: false,
            after_combinator: false,
        }
    }

    /**
     * Prepare for a simple selector, turning preceding whitespace into a descendant combinator.
     */
    fn begin_simple_selector(&mut self) {
        if self.pending_whitespace && self.in_compound && !self.after_combinator {
            self.features.insert(features::SELECTOR_DESCENDANT);
            self.finish_compound();
            self.static_text.push(' ');
        }
        self.pending_whitespace = false;
        self.after_combinator = false;
        self.in_compound = true;
    }

    /**
     * Record a simple selector that also appears in the static selector.
     */
    fn add_static_simple_selector(&mut self, feature: &'static str, text: &str) {
        self.begin_simple_selector();
        self.features.insert(feature);
        self.static_text.push_str(text);
        self.compound_simple_selectors += 1;
        self.compound_has_static_part = true;
    }

    /**
     * Close the current compound selector, keeping it matchable when only stripped parts remained.
     */
    fn finish_compound(&mut self) {
        if self.compound_simple_selectors >= 2 {
            self.features.insert(features::SELECTOR_CHAINING);
        }
        if self.in_compound && !self.compound_has_static_part {
            self.static_text.push('*');
        }
        self.in_compound = false;
        self.compound_simple_selectors = 0;
        self.compound_has_static_part = false;
    }

    /**
     * Record a combinator between compound selectors.
     */
    fn add_combinator(&mut self, feature: &'static str, symbol: char) {
        self.finish_compound();
        self.features.insert(feature);
        self.static_text.push(' ');
        self.static_text.push(symbol);
        self.static_text.push(' ');
        self.after_combinator = true;
        self.pending_whitespace = false;
    }

    /**
     * Finish the selector.
     */
    fn finish(mut self, text: &str) -> AnalysedSelector {
        self.finish_compound();
        let static_selector = self.static_text.trim().to_owned();
        AnalysedSelector {
            text: text.trim().to_owned(),
            features: self.features,
            static_selector: (self.verifiable && !static_selector.is_empty())
                .then_some(static_selector),
        }
    }
}

/**
 * Analyse every selector of a comma-separated selector list.
 */
pub(crate) fn analyse_selector_list(source: &str) -> Vec<AnalysedSelector> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut selectors = Vec::new();
    let mut builder = SelectorBuilder::new();
    let mut selector_start = parser.position();
    loop {
        let token_start = parser.position();
        let Ok(token) = parser.next_including_whitespace() else {
            break;
        };
        match token.clone() {
            Token::Comma => {
                let text = parser.slice(selector_start..token_start);
                selectors
                    .push(std::mem::replace(&mut builder, SelectorBuilder::new()).finish(text));
                selector_start = parser.position();
            }
            Token::WhiteSpace(_) => {
                if builder.in_compound {
                    builder.pending_whitespace = true;
                }
            }
            Token::Delim('>') => builder.add_combinator(features::SELECTOR_CHILD, '>'),
            Token::Delim('+') => builder.add_combinator(features::SELECTOR_ADJACENT_SIBLING, '+'),
            Token::Delim('~') => builder.add_combinator(features::SELECTOR_GENERAL_SIBLING, '~'),
            Token::Delim('*') => {
                builder.add_static_simple_selector(features::SELECTOR_UNIVERSAL, "*")
            }
            Token::Ident(name) => {
                builder.add_static_simple_selector(
                    features::SELECTOR_TYPE,
                    &serialized_identifier(&name),
                );
            }
            Token::IDHash(name) | Token::Hash(name) => {
                builder.add_static_simple_selector(
                    features::SELECTOR_ID,
                    &format!("#{}", serialized_identifier(&name)),
                );
            }
            Token::Delim('.') => match parser.next_including_whitespace() {
                Ok(Token::Ident(name)) => {
                    let text = format!(".{}", serialized_identifier(name));
                    builder.add_static_simple_selector(features::SELECTOR_CLASS, &text);
                }
                _ => builder.verifiable = false,
            },
            Token::SquareBracketBlock => {
                skip_block(&mut parser);
                let text = parser.slice_from(token_start).to_owned();
                builder.add_static_simple_selector(features::SELECTOR_ATTRIBUTE, &text);
            }
            Token::Colon => analyse_pseudo(&mut parser, token_start, &mut builder),
            _ => builder.verifiable = false,
        }
    }
    let text = parser.slice_from(selector_start);
    if !text.trim().is_empty() || !builder.features.is_empty() {
        selectors.push(builder.finish(text));
    }
    selectors
}

/**
 * Analyse the pseudo-class or pseudo-element following a colon that started at `colon_start`.
 */
fn analyse_pseudo(
    parser: &mut Parser<'_, '_>,
    colon_start: SourcePosition,
    builder: &mut SelectorBuilder,
) {
    builder.begin_simple_selector();
    let mut pseudo_element = false;
    let mut token = parser.next_including_whitespace().cloned();
    if matches!(token, Ok(Token::Colon)) {
        pseudo_element = true;
        token = parser.next_including_whitespace().cloned();
    }
    let name = match token {
        Ok(Token::Ident(name)) => name.to_ascii_lowercase(),
        Ok(Token::Function(name)) => {
            skip_block(parser);
            name.to_ascii_lowercase()
        }
        _ => {
            builder.verifiable = false;
            return;
        }
    };
    if pseudo_element || LEGACY_PSEUDO_ELEMENTS.contains(&name.as_str()) {
        if let Some(feature) = features::pseudo_element_feature(&name) {
            builder.features.insert(feature);
        }
        return;
    }
    if let Some(feature) = features::pseudo_class_feature(&name) {
        builder.features.insert(feature);
    }
    if STRUCTURAL_PSEUDO_CLASSES.contains(&name.as_str()) {
        builder.static_text.push_str(parser.slice_from(colon_start));
        builder.compound_has_static_part = true;
    }
}

/**
 * Consume the block opened by the previous token.
 */
fn skip_block(parser: &mut Parser<'_, '_>) {
    let _ = parser.parse_nested_block(|nested| {
        while nested.next_including_whitespace_and_comments().is_ok() {}
        Ok::<(), ParseError<'_, ()>>(())
    });
}

/**
 * Serialize an identifier so that it can be parsed again as part of a selector.
 */
fn serialized_identifier(name: &str) -> String {
    let mut text = String::new();
    let _ = serialize_identifier(name, &mut text);
    text
}
