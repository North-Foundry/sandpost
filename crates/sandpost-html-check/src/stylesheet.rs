//! Tolerant CSS parsing into the small rule model the detectors inspect.
//!
//! Unknown properties, vendor prefixes and invalid values are kept: email CSS is full of them and
//! the check reports what was written, not what a browser would accept.

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
};

/// Deepest rule or value nesting that is analysed; deeper content is skipped to bound recursion.
pub(crate) const MAXIMUM_NESTING_DEPTH: usize = 16;

/// One `name: value` declaration; custom property names keep their case, others are lowercase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Declaration {
    pub(crate) name: String,
    pub(crate) value: String,
}

/// A style rule or at-rule with the source text of its prelude.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CssRule {
    Style {
        selectors: String,
        declarations: Vec<Declaration>,
        nested_rules: Vec<CssRule>,
    },
    At {
        name: String,
        prelude: String,
        declarations: Vec<Declaration>,
        rules: Vec<CssRule>,
    },
}

/// Facts about the tokens of a declaration value or at-rule prelude, lowercase where case-insensitive.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ValueFacts {
    pub(crate) functions: Vec<String>,
    pub(crate) identifiers: Vec<String>,
    pub(crate) units: Vec<String>,
    pub(crate) percentage: bool,
    pub(crate) important: bool,
    pub(crate) urls: Vec<String>,
}

/// An item of a rule body.
enum BodyItem {
    Declaration(Declaration),
    Rule(CssRule),
}

/// What a block may contain.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyContent {
    Rules,
    Declarations,
    /// Declarations and nested rules, as inside a style rule.
    Mixed,
}

/// The cssparser callbacks that build [`CssRule`]s.
struct RuleCollector {
    content: BodyContent,
    depth: usize,
}

/**
 * Parse the contents of a `<style>` element into its top-level rules, skipping invalid rules.
 */
pub(crate) fn parse_stylesheet(source: &str) -> Vec<CssRule> {
    let mut parser = Parser::new(source);
    let mut collector = RuleCollector {
        content: BodyContent::Rules,
        depth: 0,
    };
    StyleSheetParser::new(&mut parser, &mut collector)
        .filter_map(Result::ok)
        .filter_map(|item| match item {
            BodyItem::Rule(rule) => Some(rule),
            BodyItem::Declaration(_) => None,
        })
        .collect()
}

/**
 * Parse a `style` attribute into its declarations, skipping invalid ones.
 */
pub(crate) fn parse_declaration_list(source: &str) -> Vec<Declaration> {
    let mut parser = Parser::new(source);
    let mut collector = RuleCollector {
        content: BodyContent::Declarations,
        depth: 0,
    };
    RuleBodyParser::new(&mut parser, &mut collector)
        .filter_map(Result::ok)
        .filter_map(|item| match item {
            BodyItem::Declaration(declaration) => Some(declaration),
            BodyItem::Rule(_) => None,
        })
        .collect()
}

/**
 * Collect the functions, identifiers, units, URLs and `!important` marker of a value.
 *
 * Nested functions and blocks are descended into, up to [`MAXIMUM_NESTING_DEPTH`].
 */
pub(crate) fn analyse_value(source: &str) -> ValueFacts {
    let mut parser = Parser::new(source);
    let mut facts = ValueFacts::default();
    collect_value_facts(&mut parser, &mut facts, 0);
    facts
}

/**
 * Record the facts of every remaining token of `input` into `facts`.
 */
fn collect_value_facts<'i>(input: &mut Parser<'i>, facts: &mut ValueFacts, depth: usize) {
    let mut after_exclamation_mark = false;
    while let Ok(token) = input.next() {
        let token = token.clone();
        let mut descend = false;
        match &token {
            Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                facts.functions.push("url".to_owned());
                let _ = input.parse_nested_block(|nested| {
                    if let Ok(Token::QuotedString(url)) = nested.next() {
                        facts.urls.push(url.to_string());
                    }
                    while nested.next().is_ok() {}
                    Ok::<(), ParseError<()>>(())
                });
            }
            Token::Function(name) => {
                facts.functions.push(name.to_ascii_lowercase());
                descend = true;
            }
            Token::ParenthesisBlock | Token::SquareBracketBlock | Token::CurlyBracketBlock => {
                descend = true
            }
            Token::Ident(name)
                if after_exclamation_mark && name.eq_ignore_ascii_case("important") =>
            {
                facts.important = true;
            }
            Token::Ident(name) => facts.identifiers.push(name.to_ascii_lowercase()),
            Token::Dimension { unit, .. } => facts.units.push(unit.to_ascii_lowercase()),
            Token::Percentage { .. } => facts.percentage = true,
            Token::UnquotedUrl(url) => facts.urls.push(url.to_string()),
            _ => {}
        }
        after_exclamation_mark = matches!(token, Token::Delim('!'));
        if descend && depth < MAXIMUM_NESTING_DEPTH {
            let _ = input.parse_nested_block(|nested| {
                collect_value_facts(nested, facts, depth + 1);
                Ok::<(), ParseError<()>>(())
            });
        }
    }
}

/**
 * Consume the rest of `input` and return its trimmed source text.
 */
fn consume_remaining(input: &mut Parser<'_>) -> String {
    let start = input.position();
    while input.next().is_ok() {}
    input.slice_from(start).trim().to_owned()
}

impl RuleCollector {
    /**
     * Parse a block body, or skip it when the nesting limit is reached.
     */
    fn parse_body(
        &self,
        content: BodyContent,
        input: &mut Parser<'_>,
    ) -> (Vec<Declaration>, Vec<CssRule>) {
        let mut declarations = Vec::new();
        let mut rules = Vec::new();
        if self.depth >= MAXIMUM_NESTING_DEPTH {
            while input.next().is_ok() {}
            return (declarations, rules);
        }
        let mut collector = RuleCollector {
            content,
            depth: self.depth + 1,
        };
        for item in RuleBodyParser::new(input, &mut collector).flatten() {
            match item {
                BodyItem::Declaration(declaration) => declarations.push(declaration),
                BodyItem::Rule(rule) => rules.push(rule),
            }
        }
        (declarations, rules)
    }
}

impl<'i> QualifiedRuleParser<'i> for RuleCollector {
    type Prelude = String;
    type QualifiedRule = BodyItem;
    type Error = ();

    /**
     * Keep the selector list as source text; it is analysed separately.
     */
    fn parse_prelude(&mut self, input: &mut Parser<'i>) -> Result<String, ParseError<()>> {
        Ok(consume_remaining(input))
    }

    /**
     * Parse a style rule body, which may nest rules.
     */
    fn parse_block(
        &mut self,
        selectors: String,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<BodyItem, ParseError<()>> {
        let (declarations, nested_rules) = self.parse_body(BodyContent::Mixed, input);
        Ok(BodyItem::Rule(CssRule::Style {
            selectors,
            declarations,
            nested_rules,
        }))
    }
}

impl<'i> AtRuleParser<'i> for RuleCollector {
    type Prelude = (String, String);
    type AtRule = BodyItem;
    type Error = ();

    /**
     * Keep the lowercase at-rule name and its prelude source text.
     */
    fn parse_prelude(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<(String, String), ParseError<()>> {
        Ok((name.to_ascii_lowercase(), consume_remaining(input)))
    }

    /**
     * Accept statement at-rules such as `@import`.
     */
    fn rule_without_block(
        &mut self,
        (name, prelude): (String, String),
        _start: &ParserState,
    ) -> Result<BodyItem, ()> {
        Ok(BodyItem::Rule(CssRule::At {
            name,
            prelude,
            declarations: Vec::new(),
            rules: Vec::new(),
        }))
    }

    /**
     * Parse an at-rule block: descriptors for `@font-face`/`@page`, otherwise nested rules (and
     * declarations when the at-rule itself is nested in a style rule).
     */
    fn parse_block(
        &mut self,
        (name, prelude): (String, String),
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<BodyItem, ParseError<()>> {
        let content = if name.ends_with("font-face") || name == "page" {
            BodyContent::Declarations
        } else if self.content == BodyContent::Mixed {
            BodyContent::Mixed
        } else {
            BodyContent::Rules
        };
        let (declarations, rules) = self.parse_body(content, input);
        Ok(BodyItem::Rule(CssRule::At {
            name,
            prelude,
            declarations,
            rules,
        }))
    }
}

impl<'i> DeclarationParser<'i> for RuleCollector {
    type Declaration = BodyItem;
    type Error = ();

    /**
     * Keep any declaration value as source text.
     *
     * Inside style rules a value containing a `{}` block is rejected so that cssparser retries the
     * input as a nested rule, as in `a:hover { color: red }`.
     */
    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _declaration_start: &ParserState,
    ) -> Result<BodyItem, ParseError<()>> {
        let name = if name.starts_with("--") {
            name.to_string()
        } else {
            name.to_ascii_lowercase()
        };
        let start = input.position();
        while let Ok(token) = input.next() {
            if self.content == BodyContent::Mixed && matches!(token, Token::CurlyBracketBlock) {
                return Err(ParseError::custom(()));
            }
        }
        let value = input.slice_from(start).trim().to_owned();
        Ok(BodyItem::Declaration(Declaration { name, value }))
    }
}

impl<'i> RuleBodyItemParser<'i, BodyItem, ()> for RuleCollector {
    /**
     * Report whether the current block may contain declarations.
     */
    fn parse_declarations(&self) -> bool {
        self.content != BodyContent::Rules
    }

    /**
     * Report whether the current block may contain nested rules.
     */
    fn parse_qualified(&self) -> bool {
        self.content != BodyContent::Declarations
    }
}
