//! Compile the backend-independent filter AST into SQLite SQL.
//!
//! SandPost stores filters as the canonical query AST, never as SQL. This module is the only
//! place that turns that AST into SQLite-specific SQL, and it lives in the SQLite backend. A
//! future backend provides its own compiler without changing the shared contract.
//!
//! The generated fragments preserve the query language's two-valued ANY-collection semantics:
//! every predicate is coerced with COALESCE so a missing value behaves like a false predicate
//! rather than SQL NULL.
use rusqlite::types::Value as SqlValue;
use sandpost_query::{Expression, Field, Operator, Predicate, Value};

/// A compiled SQL fragment with positional placeholders and their bound values.
pub(crate) struct CompiledFilter {
    /// Boolean SQL expression suitable for a WHERE clause; never evaluates to NULL.
    pub(crate) sql: String,
    /// Values bound to the fragment's placeholders, in order.
    pub(crate) parameters: Vec<SqlValue>,
}

/// Compile one canonical expression into a parameterized SQLite fragment.
pub(crate) fn compile(expression: &Expression) -> CompiledFilter {
    let mut compiler = Compiler {
        parameters: Vec::new(),
    };
    let sql = compiler.expression(expression);
    CompiledFilter {
        sql,
        parameters: compiler.parameters,
    }
}

struct Compiler {
    parameters: Vec<SqlValue>,
}

impl Compiler {
    /// Append a bound string and return its positional SQL placeholder.
    fn bind_text(&mut self, text: &str) -> String {
        self.parameters.push(SqlValue::Text(text.to_owned()));
        "?".to_owned()
    }

    /// Append a bound integer and return its positional SQL placeholder.
    fn bind_integer(&mut self, value: i64) -> String {
        self.parameters.push(SqlValue::Integer(value));
        "?".to_owned()
    }

    /// Compile a canonical expression recursively, preserving boolean composition.
    fn expression(&mut self, expression: &Expression) -> String {
        match expression {
            Expression::True => "1".to_owned(),
            Expression::False => "0".to_owned(),
            Expression::Not(operand) => format!("NOT ({})", self.expression(operand)),
            Expression::And(children) if children.is_empty() => "1".to_owned(),
            Expression::And(children) => {
                let parts: Vec<_> = children
                    .iter()
                    .map(|child| self.expression(child))
                    .collect();
                format!("({})", parts.join(" AND "))
            }
            Expression::Or(children) if children.is_empty() => "0".to_owned(),
            Expression::Or(children) => {
                let parts: Vec<_> = children
                    .iter()
                    .map(|child| self.expression(child))
                    .collect();
                format!("({})", parts.join(" OR "))
            }
            Expression::Predicate(predicate) => self.predicate(predicate),
        }
    }

    /// Dispatch a predicate to its string, numeric, or boolean representation.
    fn predicate(&mut self, predicate: &Predicate) -> String {
        match &predicate.value {
            Value::String(text) => {
                self.string_predicate(&predicate.field, predicate.operator, text)
            }
            Value::Number(number) => {
                self.numeric_predicate(&predicate.field, predicate.operator, *number)
            }
            Value::Boolean(flag) => {
                self.boolean_predicate(&predicate.field, predicate.operator, *flag)
            }
        }
    }

    /// Compile a scalar or ANY-collection string predicate with missing-value semantics.
    fn string_predicate(&mut self, field: &Field, operator: Operator, text: &str) -> String {
        match field {
            Field::Content => self.content_predicate(operator, text),
            Field::Header(name) => {
                let name_placeholder = self.bind_text(name);
                let condition = self.comparison("child.value", field, operator, text);
                self.exists(
                    "mail_headers",
                    &format!("name = {name_placeholder} AND {condition}"),
                )
            }
            _ if mailbox_column(field).is_some() => {
                let (recipient_type, column) = mailbox_column(field).expect("checked above");
                let recipient_placeholder = self.bind_text(recipient_type);
                let condition = self.comparison(&format!("child.{column}"), field, operator, text);
                self.exists(
                    "mail_recipients",
                    &format!("recipient_type = {recipient_placeholder} AND {condition}"),
                )
            }
            _ => {
                let value_sql = scalar_string_sql(field);
                let condition = self.comparison(&value_sql, field, operator, text);
                self.coalesce(&condition)
            }
        }
    }

    /// Build the boolean SQL for one string comparison, binding its literal in textual order.
    fn comparison(
        &mut self,
        value_sql: &str,
        field: &Field,
        operator: Operator,
        text: &str,
    ) -> String {
        use Operator::*;
        if text.is_empty() && matches!(operator, Contains | StartsWith | EndsWith) {
            return "1".to_owned();
        }
        let case_insensitive = match operator {
            Equal | NotEqual | Contains | StartsWith | EndsWith | Matches => {
                is_case_insensitive(field)
            }
            _ => matches!(field, Field::Content),
        };
        let placeholder = self.bind_text(text);
        let (value, pattern) = if case_insensitive {
            (
                format!("lower({value_sql})"),
                format!("lower({placeholder})"),
            )
        } else {
            (value_sql.to_owned(), placeholder)
        };
        match operator {
            Contains => format!("instr({value}, {pattern}) > 0"),
            StartsWith => format!("instr({value}, {pattern}) = 1"),
            EndsWith => format!("substr({value}, -length({pattern})) = {pattern}"),
            Matches => format!("{value} GLOB {pattern}"),
            Equal | NotEqual => {
                let operator = if matches!(operator, Equal) { "=" } else { "<>" };
                format!("{value} {operator} {pattern}")
            }
            GreaterThan | GreaterThanOrEqual | LessThan | LessThanOrEqual => {
                let operator = match operator {
                    GreaterThan => ">",
                    GreaterThanOrEqual => ">=",
                    LessThan => "<",
                    LessThanOrEqual => "<=",
                    _ => unreachable!("numeric operators are matched above"),
                };
                format!("{value} {operator} {pattern}")
            }
        }
    }

    /// Content is the union of every literal-searchable fact value.
    fn content_predicate(&mut self, operator: Operator, text: &str) -> String {
        let mut parts = Vec::new();
        for value_sql in ["mail.subject", "mail.text_body", "mail.markup_body"] {
            let condition = self.comparison(value_sql, &Field::Content, operator, text);
            parts.push(format!("COALESCE(({condition}), 0)"));
        }
        let identifier =
            self.comparison("mail.message_identifier", &Field::Content, operator, text);
        parts.push(format!("COALESCE(({identifier}), 0)"));
        for recipient_type in ["envelope_from", "envelope_to", "from", "to", "carbon_copy"] {
            let recipient_placeholder = self.bind_text(recipient_type);
            let condition = self.comparison("child.address", &Field::Content, operator, text);
            let exists = self.exists(
                "mail_recipients",
                &format!("recipient_type = {recipient_placeholder} AND {condition}"),
            );
            parts.push(exists);
        }
        for column in ["name", "value"] {
            let condition =
                self.comparison(&format!("child.{column}"), &Field::Content, operator, text);
            parts.push(self.exists("mail_headers", &condition));
        }
        format!("({})", parts.join(" OR "))
    }

    /// Compile a numeric comparison, including negative literals for unsigned facts.
    fn numeric_predicate(&mut self, field: &Field, operator: Operator, number: i64) -> String {
        let value_sql = match field {
            Field::ReceivedAt => "mail.received_at",
            Field::Size => "mail.size",
            Field::AttachmentCount => attachment_count_sql(),
            _ => return "0".to_owned(),
        };
        if is_unsigned(field) && number < 0 {
            return match operator {
                Operator::LessThan | Operator::LessThanOrEqual | Operator::Equal => "0".to_owned(),
                _ => "1".to_owned(),
            };
        }
        let placeholder = self.bind_integer(number);
        let operator = sql_operator(operator);
        self.coalesce(&format!("({value_sql}) {operator} {placeholder}"))
    }

    /// Compile attachment-presence comparisons with canonical boolean semantics.
    fn boolean_predicate(&mut self, field: &Field, operator: Operator, flag: bool) -> String {
        if !matches!(field, Field::HasAttachments) {
            return "0".to_owned();
        }
        let placeholder = self.bind_integer(i64::from(flag));
        let operator = sql_operator(operator);
        self.coalesce(&format!(
            "({}) {operator} {placeholder}",
            attachment_presence_sql()
        ))
    }

    /// Test whether any child fact satisfies the compiled collection condition.
    fn exists(&self, table: &str, condition: &str) -> String {
        self.coalesce(&format!(
            "EXISTS (SELECT 1 FROM {table} child WHERE child.mail_sequence = mail.sequence AND {condition})"
        ))
    }

    /// Force a SQL predicate to false when its value is missing.
    fn coalesce(&self, sql: &str) -> String {
        format!("COALESCE(({sql}), 0)")
    }
}

/// Return the SQL operator for a validated canonical comparison.
fn sql_operator(operator: Operator) -> &'static str {
    match operator {
        Operator::Equal => "=",
        Operator::NotEqual => "<>",
        Operator::GreaterThan => ">",
        Operator::GreaterThanOrEqual => ">=",
        Operator::LessThan => "<",
        Operator::LessThanOrEqual => "<=",
        _ => unreachable!("only comparison operators reach sql_operator"),
    }
}

/// Identify content and mailbox facts whose comparisons fold ASCII case.
fn is_case_insensitive(field: &Field) -> bool {
    matches!(field, Field::Content) || mailbox_column(field).is_some()
}

/// Identify numeric facts that cannot represent a negative value.
fn is_unsigned(field: &Field) -> bool {
    matches!(field, Field::Size | Field::AttachmentCount)
}

/// Return the correlated attachment-count expression for the current message.
fn attachment_count_sql() -> &'static str {
    "(SELECT COUNT(*) FROM mail_attachments WHERE mail_sequence = mail.sequence)"
}

/// Return the correlated attachment-presence expression for the current message.
fn attachment_presence_sql() -> String {
    format!("{} > 0", attachment_count_sql())
}

/// Map a mailbox field to its recipient role and target column.
fn mailbox_column(field: &Field) -> Option<(&'static str, &'static str)> {
    match field {
        Field::EnvelopeFromAddress => Some(("envelope_from", "address")),
        Field::EnvelopeFromDomain => Some(("envelope_from", "domain")),
        Field::EnvelopeToAddress => Some(("envelope_to", "address")),
        Field::EnvelopeToDomain => Some(("envelope_to", "domain")),
        Field::FromAddress => Some(("from", "address")),
        Field::FromDomain => Some(("from", "domain")),
        Field::ToAddress => Some(("to", "address")),
        Field::ToDomain => Some(("to", "domain")),
        Field::CarbonCopyAddress => Some(("carbon_copy", "address")),
        Field::CarbonCopyDomain => Some(("carbon_copy", "domain")),
        _ => None,
    }
}

/// Select the SQL column for a scalar string fact, or NULL for an unsupported field.
fn scalar_string_sql(field: &Field) -> String {
    match field {
        Field::Subject => "mail.subject".to_owned(),
        Field::Text => "mail.text_body".to_owned(),
        Field::MarkupBody => "mail.markup_body".to_owned(),
        Field::MessageIdentifier => "mail.message_identifier".to_owned(),
        _ => "NULL".to_owned(),
    }
}
