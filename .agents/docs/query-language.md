# Sandpost query language

The typed DSL serves scope policies, personal restrictions, saved Views, and ad hoc search. It accepts no SQL or executable code and has no HTTP or storage dependency.

## Syntax and types

```text
from.domain == "sender.test" and subject contains "password"
to.address matches "*@recipient.test" and size >= 1024
header["X-App"] == "bravo" and received_at >= 1791072000
```

Precedence, highest first: comparison, `not`, `and`, `or`; parentheses override it. Keywords and field names are case-insensitive. Quoted strings decode `\n`, `\r`, `\t`; other escapes discard the backslash. Empty source, `true`, and `false` are boolean expressions. Numeric literals are signed base-10 integers; `received_at` is Unix UTC seconds, `size` bytes, and `attachment_count` a count.

| Fields | Type / cardinality |
| --- | --- |
| `envelope.from.address`, `envelope.from.domain` | Optional string |
| `envelope.to.address`, `envelope.to.domain`; `from`, `to`, `cc` `.address`/`.domain` | Collections |
| `subject`, `text`, `html` | String |
| `content` | Collection of subject, text, HTML, envelope/MIME addresses, RFC Message-ID, and header names/values |
| `message_id` | Optional RFC Message-ID, distinct from public UUID |
| `header["name"]` | Collection of values; lookup name is lowercase |
| `received_at`, `size`, `attachment_count` | Integer |
| `has_attachments` | Boolean |

`envelope.*` comes from SMTP; plain address fields come from MIME and can differ. Aliases `envelope.from`, `envelope.to`, `from`, `to`, and `cc` select those addresses; there is no `bcc`. Strings allow `==`, `!=`, `contains`, `starts_with`, `ends_with`, `matches`; integers also allow `>`, `>=`, `<`, `<=`; booleans only `==`, `!=`. Subject, text, HTML, content, message ID, and header values allow lexicographic ordering; mailbox fields do not. Invalid fields, type/operator mismatches, incomplete input, and trailing tokens report zero-based UTF-8 byte positions.

Domains are normalized at ingest. SMTP envelope local-parts retain spelling/case; MIME addresses are lowercased. Mailbox addresses/domains compare ASCII-case-insensitively, including `matches`. Subject, text, HTML, message ID, and header values are case-sensitive; header names are trimmed and ASCII-lowercased for lookup. `content` includes header names and values and compares all values ASCII-case-insensitively, also for ordering; non-ASCII characters are unchanged. Missing optional fields and empty collections have no values, so `!=` is false for them.

## Collections, globs, and numbers

Collection comparisons use **ANY**, including repeated headers. Thus `to.domain != "recipient.test"` means some recipient differs and may be true even if another equals it. To require that none match, use `not (to.domain == "recipient.test")`. Negation applies outside the existential comparison; equality and inequality are not logical complements.

`matches` is a whole-value glob: `*` matches any number of Unicode scalar values, `?` one, and other characters are literal. An empty pattern matches only an empty value. Backslash cannot escape glob metacharacters because string decoding removes it. Globs are not regex and have no character classes or regex backtracking. Ordering is Unicode lexicographic except `content`, which folds ASCII letters first. Evaluation stays exact under resource limits, including beneath `not`.

Literals are signed `i64` with no separators or unit suffixes. Underlying sizes/counts are unsigned: evaluation compares them exactly with signed literals, while public `Value::Number` saturates values above `i64::MAX`. Use predicates/evaluation for exact decisions, not that projection.

## Canonicalization and search

AND/OR flatten, sort, and deduplicate; constants fold; double negation collapses; exact expression/NOT-expression pairs simplify. Operand reordering preserves canonical form/fingerprint. This is practical normalization, not a theorem prover; distributive equivalence is not exhaustively computed. Fingerprints use format-v1 SHA-256 over tagged, length-delimited canonical bytes; existing field tags stay stable.

Tantivy plans safe candidate supersets using exact-value terms and trigrams. Boolean constants, supported numeric comparisons, attachment-presence comparisons, and AND/OR compositions of these exact predicates use indexed columns directly. Other candidates require canonical-expression verification against derived-index facts. Oversized encoded terms broaden candidates instead of excluding matches. Short substrings may have no useful trigrams, and broad negation has no selective positive candidate; these can scan broadly but cannot change semantics. Never approximate negation by complementing an incomplete index.

SQL hydration verifies persistent message revisions, rechecks the composed filter against current facts, and excludes the caller's user-message tombstones. Search refills bounded candidate pages after exclusions and advances by indexed sequences even past missing physical rows. Tombstones are separate user state, not query fields or per-user index content. Direct reads authorize facts and payload in one current SQL snapshot. See [storage.md](storage.md) for outbox, revision, hydration, and derived-index lifecycle details.

Compilation limits: source 16 KiB (16,384 UTF-8 bytes), 4,096 tokens, and nesting depth 64 for parentheses/`not`. Glob matching is never truncated when expensive; a pattern is bounded by the source limit.

HTTP `/api/v1/search` `q` always contains DSL. The client literal-search adapter encodes entered text as `content contains <DSL-quoted string>`; the native helper builds the equivalent canonical `Content` predicate. The Vue search box additionally rewrites `from:`, `to:`, `cc:`, `subject:`, `has:attachment`, `after:`, and `before:` shortcuts into canonical predicates. These shortcuts are not DSL and are not accepted by Views, scopes, or the API. Views and personal filters use the same DSL; a View organizes mail and grants no authorization.

The compiler indexes raw HTML with substring semantics; it does not strip tags or add token/stemming behavior. Attachment counts are indexed, while filenames and full metadata stay in authoritative storage; filename search is not exposed. Bcc recipients are searchable through SMTP envelope recipients and captured header values, with no separate Bcc field.
