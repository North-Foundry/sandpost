# Repository conventions

These instructions apply to every agent working in this repository. The root
`AGENTS.md` is the single project instruction entry point.

## Commit messages

Use [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/)
for every commit:

```text
<type>(<optional-scope>)<optional-!>: <description>

<optional body>

<optional footer(s)>
```

- Use one of these types:
    - `feat`: a user-visible feature
    - `fix`: a bug fix
    - `docs`: documentation-only changes
    - `refactor`: code changes that neither fix a bug nor add a feature
    - `perf`: performance improvements
    - `test`: adding or correcting tests
    - `build`: build system or dependency changes
    - `ci`: continuous-integration changes
    - `style`: formatting with no semantic change
    - `chore`: repository maintenance not covered above
    - `revert`: reverting an earlier commit
- Write the description in imperative mood, starting with a lowercase letter
  or digit and without a trailing period. Keep the complete first line at or
  below 72 characters.
- Use a short, lowercase, kebab-case scope when it adds useful context, for
  example `core`, `syntax`, `parser`, `cli`, or `linear-measure`.
- Add `!` before `:` for a breaking change and explain the impact in a
  `BREAKING CHANGE:` footer.
- Separate a non-empty body or footer from the subject, and each other, with a
  blank line. Explain motivation and consequences rather than restating the
  diff.
- Keep commits focused. Do not use vague subjects such as `update files`,
  `changes`, or `fix stuff`.

Examples:

```text
feat(parser): support qualified unit literals
fix(core): reject duplicate operator signatures
docs: explain extension registration
refactor(runtime)!: replace the value dispatch contract
```

Agents must propose and create commit messages that follow this convention,
even when they are not responsible for running `git commit` themselves.

The repository provides a versioned `commit-msg` hook. Enable it once per
clone with:

```sh
git config core.hooksPath .githooks
```
