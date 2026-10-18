# Code style

## TypeScript sources

Author JavaScript-platform source code in TypeScript, including application
code, Electron main/preload code, shared packages, Vite plugins, configuration,
build/development scripts and tests. Use `.ts`, `.mts` or `.cts` as appropriate
and TypeScript script blocks in Vue single-file components. Do not maintain
handwritten `.js`, `.mjs` or `.cjs` source files.

When a runtime or tool requires JavaScript, compile the TypeScript sources and
configure it to consume the generated output. Keep emitted JavaScript in ignored
build directories; do not check it in or maintain it as a second source copy.
Where the supported runtime loads TypeScript directly, use that capability and
keep a separate TypeScript check because direct execution does not type-check.

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

Document every project-defined function and method with the language's doc
comment syntax, including private helpers and test functions. At minimum, state
the function's responsibility. When behavior is not evident from the signature,
also explain the relevant representation, conversion, ordering, error, or
side effect semantics.

Always format function and method documentation as a multiline documentation
block, even when its text is a single line or sentence. Put the opening `/**`,
each content line prefixed with ` * `, and the closing ` */` on separate lines.
This applies to TypeScript/JSDoc and Rust doc blocks, including helpers and tests.
Do not collapse function documentation into `/** Description. */`.

```typescript
/**
 * Publish the development server origin.
 */
```

Prefer concise comments that add useful context, but do not omit documentation
merely because a function is small or its name is descriptive. Keep detailed
implementation commentary next to the specific algorithm or invariant it
explains.
