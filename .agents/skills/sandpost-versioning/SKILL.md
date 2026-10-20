---
name: sandpost-versioning
description: Choose and apply Sandpost's next version when completing product changes or preparing a release. Evaluate accumulated release scope, reuse pending versions, and distinguish alpha increments, significant minor changes, and readiness for the first stable 1.0.0.
---

# Sandpost version selection

Read [the versioning policy](../../docs/versioning.md) for decision criteria,
maturity gates, shared manifests and release mechanics. Treat it as the source
of truth; honor any explicit user version instruction.

1. Inspect the working tree before editing. Read `[workspace.package].version`
   in `Cargo.toml`, `apps/desktop/package.json`, relevant version tags and their
   commits. Establish the release baseline from available release evidence;
   distinguish a reserved/tagged version from a published release. Inspect all
   changes intended for the pending release, including committed and uncommitted
   changes. Do not include unrelated local work or edit it to make a release fit.
2. Apply the policy to that complete scope. Decide whether to leave the version
   unchanged, reuse its pending value, increment the prerelease counter, or raise
   the core version. For a new version, check that the candidate exceeds the
   baseline, never lowers the current selection, and has no existing tag. Decide routine alpha/minor
   changes autonomously; do not ask the user to choose an increment. Apply
   maturity gates separately from change size. If release history or scope
   cannot be established, report what is missing instead of guessing.
3. When a bump is needed within the task's scope, update the shared Rust and
   desktop manifest versions together. Refresh the affected workspace package
   versions and references in `Cargo.lock` with Cargo, preserving unrelated
   dependency versions. Inspect the lockfile diff for unintended dependency
   changes. Resolve only version drift relevant to this task. Documentation-only
   changes to this policy or skill do not themselves require version alignment.
4. Check manifest agreement and run `cargo metadata --locked --format-version 1`
   to confirm that Cargo accepts the workspace and lockfile
   without changes. Run checks relevant to the product change and artifact scope
   as required by the project; version edits alone do not warrant the full test
   suite. Do not claim checks passed if unavailable or blocked.
5. Report the release baseline, old and selected version, reason, and verification
   performed. If no bump is needed or the pending version is reused, say so.
   Leave website download metadata at the published release until publication.

The skill selects local versions. Commit, tag, push and publication actions
follow the user's authorization and the documented release workflow; invoking
this skill alone does not request or authorize those actions.
