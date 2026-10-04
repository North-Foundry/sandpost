# Sand Post Query Language

One typed DSL serves scope policies, personal restrictions and future saved
inboxes/search. It accepts no SQL or executable code and has no HTTP/storage
dependency.

```text
source -> lexer -> parser -> typed AST / validation
       -> normalization / simplification -> canonicalization / fingerprint
       -> predicates + cost hints -> shared matching nodes
```

## Syntax and types

```text
from.domain == "bflow.dev" and to.domain == "boris.it"
from.address == "dev@bflow.dev" and subject contains "password"
(from.domain == "bflow.dev" or from.domain == "website.dev")
    and to.address matches "*@boris.it"
header["X-App"] == "bflow"
size >= 1024 and has_attachments == true
received_at >= 1791072000
```

Precedence, highest first: comparison, `not`, `and`, `or`. Parentheses override
it. Keywords and field names are case insensitive. Single/double quoted strings
accept `\n`, `\r`, and `\t`; for any other escaped character the backslash is
discarded and the character is kept. Empty source, `true` and `false` are
boolean expressions. Numeric literals are signed integers; `received_at` uses
Unix UTC seconds, `size` uses bytes, and `attachment_count` is an integer.

| Fields | Type / cardinality |
| --- | --- |
| `envelope.from.address`, `envelope.from.domain` | Optional string |
| `envelope.to.address`, `envelope.to.domain` | Collection of strings |
| `from.address`, `from.domain`, `to.address`, `to.domain`, `cc.address`, `cc.domain` | Collections of strings |
| `subject`, `text`, `html` | String |
| `message_id` | Optional string (RFC Message-ID, distinct from public UUID) |
| `header["name"]` | Collection of header values, lowercase key |
| `received_at`, `size`, `attachment_count` | Integer |
| `has_attachments` | Boolean |

`envelope.*` fields come from the SMTP transaction, while plain `from`, `to`,
and `cc` fields come from the message's MIME address headers; the two sources
can differ. `message_id` is the MIME Message-ID, not Sand Post's public UUID.

String operators: `==`, `!=`, `contains`, `starts_with`, `ends_with`, `matches`.
Numeric operators: `==`, `!=`, `>`, `>=`, `<`, `<=`. Boolean operators: `==`,
`!=`. Subject/body/message-ID/header strings additionally support lexicographic
`>`, `>=`, `<`, `<=`; mailbox fields reject ordered comparisons. Invalid fields,
operator/type combinations, incomplete input and trailing tokens return errors
with zero-based UTF-8 byte positions. Missing optional fields and empty
collections produce no matching values, so even `!=` is false for them.
Mailbox addresses and domains are normalized at ingest, and mailbox comparisons
are ASCII-case-insensitive. `matches` uses that same mailbox case rule.
Subject/body/header value comparisons are case sensitive. Header names are
trimmed and ASCII-lowercased for lookup.

## Collection semantics

**ANY** applies to comparisons: `to.domain == "boris.it"` is true if any
recipient's domain equals that value. `to.domain != "boris.it"` means any
recipient has another domain, and can be true even alongside an equal recipient.
To express no recipient in that domain use:

```text
not (to.domain == "boris.it")
```

This distinction also applies to duplicate headers. Negation is outside the
collection existential operation. Therefore equality and inequality are not
logical complements for collections and are never simplified as complements.

`matches` is a whole-value glob: `*` matches any number of Unicode scalar
values, `?` one scalar value. Other characters are literals. An empty pattern
matches only an empty value. Backslash does not escape `*` or `?`; string
decoding removes the slash before glob matching. The glob is not regex and does
not implement character classes or regex backtracking. Pattern length is
bounded by query limits. The evaluator is exact; it must not silently return a
different boolean when a resource limit is hit, particularly underneath `not`.

## Canonical forms and safety

AND/OR flatten, sort and deduplicate children. Constants fold, double negation
collapses and exact expression/NOT-expression pairs simplify. Equivalent operand
order produces the same canonical fingerprint. This is practical normalization,
not a theorem prover: distributive equivalence is not exhaustively computed.
Fingerprints use SHA-256 over a versioned, tagged, length-delimited encoding
(`sandpost-query` format v1), rather than Rust debug formatting. They are
compiler-format identifiers, not permanent authorization tokens;
format changes require rebuilding policy snapshots/materializations.

The source limit is 16 KiB, token limit 4096 and nesting limit 64. The parser
rejects oversized expressions before deep recursion. Typed ASTs are public
library data; compiled queries expose their expression immutably, while callers
constructing ASTs directly must uphold the same validation/normalization
invariant before using them as policies.

Cheap exact mailbox/header predicates supply inverted-index anchors. Cost hints
order cheap work ahead of body/glob work. The matcher interns equivalent
predicates and expressions to avoid duplicated runtime execution.

Glob-to-domain rewriting is deliberately deferred. `*@domain` is not equivalent
to a domain test unless mailbox normalization, glob grammar and address
validity establish all preconditions; an unsafe optimization could change
authorization. Future optimizer passes must prove equivalence on scalar and
collection cases and preserve missing-value/negation semantics.
