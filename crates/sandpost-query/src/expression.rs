//! Typed query expressions shared with the matcher.

/// A field from normalized message facts; mailbox and header fields may have many values.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    EnvelopeFromAddress,
    EnvelopeFromDomain,
    EnvelopeToAddress,
    EnvelopeToDomain,
    FromAddress,
    FromDomain,
    ToAddress,
    ToDomain,
    CarbonCopyAddress,
    CarbonCopyDomain,
    Subject,
    Text,
    MarkupBody,
    MessageIdentifier,
    ReceivedAt,
    Size,
    AttachmentCount,
    HasAttachments,
    Header(String),
}

/// A typed comparison, or a whole-value string matching operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Operator {
    Equal,
    NotEqual,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
    Contains,
    StartsWith,
    EndsWith,
    Matches,
}

/// A literal on the right-hand side of a comparison.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Value {
    String(String),
    Number(i64),
    Boolean(bool),
}

/// One comparison; it succeeds if any value of the field satisfies it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Predicate {
    pub field: Field,
    pub operator: Operator,
    pub value: Value,
}

/// A boolean expression. Prefer [`crate::compile`] for validated, bounded policies.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Expression {
    True,
    False,
    Predicate(Predicate),
    Not(Box<Expression>),
    And(Vec<Expression>),
    Or(Vec<Expression>),
}

impl Field {
    /// Report whether this field contains mailbox addresses or domains.
    pub(crate) fn is_mailbox_field(&self) -> bool {
        matches!(
            self,
            Field::EnvelopeFromAddress
                | Field::EnvelopeFromDomain
                | Field::EnvelopeToAddress
                | Field::EnvelopeToDomain
                | Field::FromAddress
                | Field::FromDomain
                | Field::ToAddress
                | Field::ToDomain
                | Field::CarbonCopyAddress
                | Field::CarbonCopyDomain
        )
    }
}
