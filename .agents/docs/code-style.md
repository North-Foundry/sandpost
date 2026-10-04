# Code style

## Naming

Use complete, domain-specific English names for project-defined identifiers.
A reader should understand an identifier without having to expand an
abbreviation from its context. The same naming rules apply to SQL table names,
column names, and references between tables.

- Spell out the words that describe an identifier's purpose. For example, use
  `equality` instead of `eq` and `partial_equality` instead of `partial_eq`.
- Do not introduce shortened words, initialisms, or local acronyms merely
  because their meaning can be inferred from nearby code.
- Choose a name that states the identifier's role and domain, not just its
  representation or position in an expression.

The only exception is an identifier imposed directly by the Rust core or a
third-party library. Keep that external identifier unchanged when the API
requires it, but do not carry its abbreviation into project-defined names.

## Documentation

Document every project-defined function and method with a Rust doc comment,
including private helpers and test functions. At minimum, state the function's
responsibility. When behavior is not evident from the signature, also explain
the relevant representation, conversion, ordering, error, or side-effect
semantics.

Prefer concise comments that add useful context, but do not omit documentation
merely because a function is small or its name is descriptive. Keep detailed
implementation commentary next to the specific algorithm or invariant it
explains.
