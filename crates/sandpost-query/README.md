# `sandpost-query`

`sandpost-query` compiles a small, typed filter language for Sand Post message
facts. It has no SQL, executable-code, HTTP, or storage support. The crate is
infrastructure-independent; `compile` validates a source string and returns an
immutable canonical query that can be evaluated against `sandpost_core::MessageFacts`.

```rust
use sandpost_core::MessageFacts;
use sandpost_query::compile;

let facts = MessageFacts {
    subject: "Build report".into(),
    ..MessageFacts::default()
};
let query = compile(r#"subject starts_with "Build" and size < 2048"#)?;
assert!(query.evaluate(&facts));
let canonical_expression = query.expression();
# let _ = canonical_expression;
# Ok::<(), sandpost_query::QueryError>(())
```

## Language

A predicate compares a field with a literal: `size >= 1024` has field `size`,
operator `>=`, and integer value `1024`. Boolean expressions combine predicates
with `and`, `or`, and `not`, or use the constants `true` and `false`.

An empty or whitespace-only source is `true`. The literals `true` and `false`
are also boolean expressions. Comparisons bind most tightly, followed by
`not`, `and`, then `or`; parentheses override this order. Keywords and field
names are case-insensitive.

```text
from.domain == "example.org" and subject contains "invoice"
(to.address matches "*@example.org" or header["X-Route"] == "archive")
not has_attachments == true or size >= 1048576
```

### Fields and types

All fields below accept the listed string or scalar type. Collection fields
contain zero or more values and use the same `ANY` rule described below.

| DSL field | Type and values |
| --- | --- |
| `envelope.from.address` | Optional string: SMTP envelope sender address. `envelope.from` is an alias. |
| `envelope.from.domain` | Optional string: SMTP envelope sender domain. |
| `envelope.to.address` | Collection of strings: SMTP envelope recipient addresses. `envelope.to` is an alias. |
| `envelope.to.domain` | Collection of strings: SMTP envelope recipient domains. |
| `from.address` | Collection of strings parsed from the MIME `From` header. `from` is an alias. |
| `from.domain` | Collection of domains parsed from MIME `From`. |
| `to.address` | Collection of strings parsed from MIME `To`. `to` is an alias. |
| `to.domain` | Collection of domains parsed from MIME `To`. |
| `cc.address` | Collection of strings parsed from MIME `Cc`. `cc` is an alias. |
| `cc.domain` | Collection of domains parsed from MIME `Cc`. |
| `subject` | String; empty if no subject was parsed. |
| `text` | String formed by joining parsed text body parts with newlines. |
| `html` | String formed by joining parsed HTML body parts with newlines. |
| `content` | Collection of subject, text, HTML, envelope and MIME addresses, RFC Message-ID, and header names and values. |
| `message_id` | Optional RFC Message-ID string, distinct from Sand Post's public message UUID. |
| `header["name"]` | Collection of values for a header name; duplicate header values are preserved. |
| `received_at` | Signed integer: Unix UTC seconds. |
| `size` | Nonnegative integer: raw message size in bytes. |
| `attachment_count` | Nonnegative integer: parsed attachment count. |
| `has_attachments` | Boolean, true when attachment count is greater than zero. |

The `envelope.*` fields come from SMTP's `MAIL FROM` and `RCPT TO` transaction.
The plain `from`, `to`, and `cc` fields come from the message's MIME address
headers; they can differ from the envelope. There is no `bcc` query field.
`message_id` reads the MIME Message-ID header, not the UUID assigned by Sand
Post.

### Operators and precedence

The ten comparison operators are:

| Operator | Meaning | Example |
| --- | --- | --- |
| `==` | Equal | `from.domain == "example.org"` |
| `!=` | Not equal, with collection `ANY` semantics | `to.domain != "example.org"` |
| `>` | Greater than | `size > 1024` |
| `>=` | Greater than or equal | `received_at >= 1791072000` |
| `<` | Less than | `attachment_count < 2` |
| `<=` | Less than or equal | `subject <= "M"` |
| `contains` | String contains the literal substring | `subject contains "invoice"` |
| `starts_with` | String begins with the literal prefix | `subject starts_with "Build"` |
| `ends_with` | String ends with the literal suffix | `subject ends_with "report"` |
| `matches` | Whole-string glob match | `to.address matches "*@example.org"` |

Mailbox fields (`envelope.*`, `from.*`, `to.*`, `cc.*`) accept `==`, `!=`,
`contains`, `starts_with`, `ends_with`, and `matches`. `subject`, `text`, `html`,
`content`, `message_id`, and `header["name"]` accept those string operators plus ordered
comparisons `>`, `>=`, `<`, and `<=`. Numeric fields (`received_at`, `size`, and
`attachment_count`) accept the six symbolic comparisons with an integer.
`has_attachments` accepts only `==` and `!=` with `true` or `false`. String
ordering compares Unicode strings lexicographically.

`content` uses collection `ANY` semantics and ASCII-case-insensitive string
comparisons, including ordered comparisons; non-ASCII characters are unchanged.
Its values include header names as well as header values.

Boolean precedence, highest first, is comparison, `not`, `and`, then `or`.
`not` is unary; `and` and `or` combine two expressions (and may be repeated).
For example:

```text
not subject == "draft" or has_attachments == true and size >= 1048576
```

groups as:

```text
(not (subject == "draft")) or
    ((has_attachments == true) and (size >= 1048576))
```

Parentheses can change the grouping, for example
`not (subject == "draft" or subject == "temporary")`. Numeric literals are
signed base-10 `i64` integers with no separators or suffixes. Use
`size >= 1048576` for one MiB; there are no size-unit suffixes. A `received_at`
literal is a Unix timestamp in seconds, not a date string or milliseconds.

### Collection and missing-value behavior

Every field comparison uses existential `ANY` semantics: a collection predicate
is true when at least one present value satisfies that comparison. This makes
`!=` different from negated equality:

```text
to.domain != "example.org"
```

is true if any recipient has a domain other than `example.org`, even if another
recipient is in `example.org`. To require that no recipient has that domain,
negate the equality predicate:

```text
not (to.domain == "example.org")
```

The same rule applies to duplicate header values and all other collections.
For an empty collection or a missing optional value, every positive comparison
is false—including `!=`; negating such a comparison is therefore true. Scalar
string fields (`subject`, `text`, and `html`) are always present in
`MessageFacts`, though they may be empty strings.

### Strings, case, and glob patterns

Single- and double-quoted strings are accepted. Backslash escapes translate
`\n`, `\r`, and `\t` to newline, carriage return, and tab. For every other
escaped character, the backslash is discarded and that next character is kept;
for example, `\"` becomes a quote and `\\` becomes one backslash. A trailing
backslash or missing closing quote is an error. These are string-literal
escapes, not regular-expression escapes.

Field names and keywords are case-insensitive. Header names are trimmed and
ASCII-lowercased for lookup. Mailbox values are normalized to lowercase during
ingest, and mailbox comparisons are ASCII-case-insensitive; the compiler also
lowercases mailbox string literals. Content comparisons are ASCII-case-insensitive
and leave non-ASCII characters unchanged. Other string values, including header
values, are case-sensitive. `contains`, `starts_with`, and `ends_with` on
mailbox fields use ASCII-case-insensitive matching. Mailbox `matches` patterns
use the same ASCII-case-insensitive character comparison. `content` `matches`
patterns apply that rule to ASCII characters and preserve non-ASCII characters.

`matches` is a whole-string glob, not a regular expression:

| Pattern | Meaning |
| --- | --- |
| `*` | Zero or more Unicode scalar values |
| `?` | Exactly one Unicode scalar value |
| Any other character | That literal character |

For example, `subject matches "Build *"` matches the whole subject beginning
with `Build `, and `subject matches "?"` matches a subject containing exactly
one Unicode scalar value. There are no character classes or regex operators.
Backslash does not quote glob metacharacters: after string decoding, `*` and `?`
remain glob operators. An empty pattern matches only an empty value; `*`
matches any value, including the empty string. Matching has no silent work
cutoff: the greedy matcher is exact and has worst-case work proportional to
text length times pattern length.

## Rust API

The crate root re-exports `compile`, `CompiledQuery`, `QueryError`, `Expression`,
`Field`, `Operator`, `Predicate`, and `Value`. Implementation modules are
private; downstream code uses the root exports.

| Public type | Represents |
| --- | --- |
| `Field` | A field to read from message facts |
| `Operator` | A comparison or string matching operation |
| `Value` | A string, signed integer, or boolean literal |
| `Predicate` | One field/operator/value comparison |
| `Expression` | Constants, predicates, and boolean combinations |
| `CompiledQuery` | A validated canonical expression and its fingerprint |
| `QueryError` | A diagnostic message and source byte position |

Rust identifiers use full names, including `Field::CarbonCopyAddress`,
`Field::MarkupBody`, `Field::MessageIdentifier`, `Operator::GreaterThanOrEqual`,
and `Value::Boolean`; `Predicate` exposes its operator as `operator`. The DSL
spellings remain `cc`, `html`, and `message_id`.

| Module | Responsibility |
| --- | --- |
| `src/lib.rs` | Public root exports and module declarations |
| `src/compiler.rs` | `compile`, source-size limit, diagnostics, immutable compiled query |
| `src/lexer.rs` | `tokenize`, token kinds, and UTF-8 byte positions |
| `src/parser.rs` | Grammar, precedence, field/operator/type validation |
| `src/expression.rs` | Public AST, field, operator, predicate, and value types |
| `src/canonical.rs` | Boolean simplification and v1 fingerprint encoding |
| `src/evaluation.rs` | Predicate evaluation, field values, and cost hints |
| `src/glob.rs` | Whole-value Unicode glob matching |
| `src/tests/` | Unit tests, grouped by behavior |
| `tests/` | Public-API integration tests |

`CompiledQuery` is immutable through its public API, so its canonical expression
cannot be changed while retaining a stale fingerprint. Use `expression()` to
borrow the AST, `into_expression()` to consume the query and take the AST, and
`fingerprint()` to borrow its fingerprint. It also provides `evaluate(&facts)`.
The AST types and fields are public for matcher and tooling integration. Code
that constructs an `Expression` or `Predicate` directly bypasses source validation;
such ASTs are trusted data and callers must uphold the compiler's field/type
rules and canonical-form invariant before using them as a policy. Prefer
`compile` for untrusted query text.

`Field::values(&facts)` and `Predicate::field_values(&facts)` return an owned
`Vec<Value>` projection. Since `Value::Number` is `i64`, a `size` or
`attachment_count` above `i64::MAX` is represented there as `i64::MAX`;
`Predicate::evaluate` compares those unsigned facts exactly against signed DSL
literals. Use evaluation for exact numeric decisions rather than relying on the
saturated projection.

Compilation errors are `QueryError { position, message }`, displayed as
`<message> at byte <position>`. Positions are zero-based UTF-8 byte offsets,
not character indexes. Errors cover unknown fields, invalid field/operator or
literal/type combinations, missing syntax, unexpected trailing tokens,
unterminated strings/escapes, integer overflow, and limit violations. The
message text is diagnostic; callers needing structured recovery should retain
the position and treat the text as human-readable.

```rust
use sandpost_query::compile;

let source = "subject ==";
let error = compile(source).unwrap_err();
assert_eq!(error.position, source.len());
assert!(error.to_string().contains("at byte"));
```

`Predicate::cost()` returns a relative planning hint, not a time or a guarantee:

| Cost | Fields |
| --- | --- |
| `1` | `message_id`, `received_at`, `size`, `attachment_count`, `has_attachments` |
| `2` | `envelope.from.*`, `from.*` |
| `4` | `header["name"]` |
| `6` | All remaining fields, including recipient fields and body text |

The matching layer can use cheap exact predicates as candidate anchors and
order work using these hints. See the
[architecture guide](../../docs/architecture.md) for crate boundaries and the
[query-language reference](../../docs/query-language.md) for the compact DSL
reference and safety rationale.

## Normalization, fingerprints, and limits

Compilation flattens nested `and`/`or`, sorts and deduplicates their children,
folds boolean constants, collapses double negation, and simplifies an exact
expression paired with its explicit negation. This is practical normalization,
not a theorem prover; distributive equivalence is not expanded. It never treats
collection `!=` as the complement of `==`.

`fingerprint()` is lowercase hexadecimal SHA-256 over a tagged,
length-delimited canonical encoding prefixed by `sandpost-query` format v1. It
does not hash Rust debug output. Equivalent canonical operand order produces
the same fingerprint. Fingerprints identify a compiler encoding; they are not
permanent authorization tokens. If the encoding changes, rebuild policy
snapshots and materializations that store fingerprints.

| Limit | Maximum |
| --- | ---: |
| Source | 16 KiB (16,384 UTF-8 bytes) |
| Tokens | 4,096 |
| Parenthesis/`not` nesting | 64 |

Compilation enforces these bounds before accepting a query. The glob pattern
is part of the bounded source, while the message value can be longer; matching
remains exact rather than returning a different result when work is large.

Run the crate's unit, integration, and documentation examples (including
doctests sourced from this README) with:

```sh
cargo test -p sandpost-query
```
