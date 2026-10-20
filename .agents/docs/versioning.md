# Versioning and releases

Sandpost uses one shared version for its Rust crates, CLI/runtime and desktop app. Set the Rust version in `[workspace.package]` in the root `Cargo.toml`; Rust crates inherit it. Keep `apps/desktop/package.json` at the same version. Private packages used only for unrelated tooling do not need to share it. `Cargo.lock` must reflect the release manifests and be committed with them. The initial release version was `0.1.0-alpha.1`; the first stable release will be `1.0.0`.

## Agent version selection

Use the repository skill [sandpost-versioning](../skills/sandpost-versioning/SKILL.md)
when completing product changes or preparing a release. Agents choose and apply
the next development version autonomously using the criteria below; an explicit
user instruction about the version takes precedence. Select by product impact,
not changed-line count, elapsed time, commit count, or commit type alone.

During initial development, use `0.MINOR.0-alpha.N`. Keep the patch at zero and
increment only one level for the complete release change set:

| Change since the previous release | Next version from `0.1.0-alpha.2` |
| --- | --- |
| Bug fixes, small UI improvements, optimizations, internal refactors, or minor extensions to an existing feature | `0.1.0-alpha.3` |
| A substantial new user capability, workflow, integration or distribution method; or an incompatible change to an exposed contract | `0.2.0-alpha.1` |
| Documentation, tests, formatting, or maintenance without a product behavior or distribution change | No bump |

For example, fixing an inbox badge or improving an existing search control
increments alpha. Introducing desktop distribution, a new authentication method,
or a new mail protocol increments the minor. Changes to API, CLI, configuration,
query syntax, or persisted-data compatibility count as exposed contract changes;
an internal implementation rewrite with preserved behavior does not. A small
optional API extension does not automatically qualify as a substantial feature.
Dependency and build changes use their actual product or distribution impact.
For mixed changes, apply the highest applicable level once. When impact is
unclear, choose the smaller increment and explain the assumption; never
downplay a known incompatible contract change.

Assess the accumulated changes since the preceding release, including changes
already committed for the pending release and the current task. Inspect the
manifests, relevant version tags and their commits, and the diff; the highest
local tag alone does not establish that a GitHub release was published. If the
manifest still names a released or tagged version, reserve the next unused
version. If it already names an unreleased, untagged version covering these
changes, reuse it across tasks; raise its minor only if the accumulated scope
now requires that. Do not increment alpha again merely for another prompt,
commit, review fix or CI retry in the same pending release. Never lower an
already selected version, reuse an existing tag, or alter a published release.
If the release baseline cannot be established from available evidence, report
the uncertainty rather than inventing release history or changing versions
speculatively.

Record the old and selected version, the release baseline, and a short reason in
the completion report, including when the pending version is reused or no bump
is needed. Keep this policy in the documentation and the execution procedure in
the skill. Choosing or updating a local version does not authorize committing,
tagging, pushing or publishing; follow the user's scope and the release workflow
below for those actions. Website download metadata continues to describe only
published releases, not the pending manifest version.

## First stable release and later versions

An illustrative development path is `0.1.0-alpha.2`, `0.1.0-alpha.3`,
`0.2.0-alpha.1`, then `1.0.0-alpha.1` if further alpha work is needed,
`1.0.0-beta.1`, `1.0.0-rc.1`, and stable `1.0.0`. Intermediate minors and stages
are optional; there is no requirement to reach `0.9.0`. Do not publish a stable
`0.x` release. Reset the numeric prerelease counter to 1 when the core version or stage
changes. Use numeric identifiers, never a bare `alpha`, `beta` or `rc`.

Promote maturity based on evidence and the agreed release scope, not the size
or number of changes:

- Target `1.0.0` once the first stable release scope has been agreed. Further
  prereleases of that target keep the `1.0.0` core version.
- Beta means that the planned V1 capabilities are implemented and ready for
  feedback from real use.
- Release candidate means that no known release blockers remain and the checks
  relevant to the declared artifacts and platforms pass.
- Stable means that the agreed acceptance criteria pass: core SMTP capture,
  reading, search and permissions work reliably; supported upgrades preserve
  data through verified migrations with documented backup/recovery; exposed
  compatibility contracts are documented; installation and upgrade work on
  declared supported platforms. Document preview components and exclusions
  explicitly rather than implying all distributions have equal maturity.

Do not infer a promotion from green tests alone or automatically promote to
stable as part of an unrelated change. While remaining at the same stage and
target version, increment only its counter for each new release, for example
`1.0.0-beta.1` to `1.0.0-beta.2`. Significant changes during beta or RC require
renewed validation; they do not justify skipping directly to stable or selecting
`1.1.0` before V1 ships. Never move backwards in version precedence.

After `1.0.0`, follow [Semantic Versioning](https://semver.org/): compatible bug
fixes increment patch (`1.0.1`), compatible features increment minor (`1.1.0`),
and incompatible exposed contract changes increment major (`2.0.0`). Prereleases
of a future stable release use that target version with `alpha.N`, `beta.N` or
`rc.N` as appropriate.

## Release mechanics

The Git tag is `v` plus the exact manifest version, such as
`v0.2.0-alpha.1`; the `v` belongs only in the tag. Never move or recreate a
published tag. Fix a published release by making a new version and tag.

The CLI release lockfile must match the committed workspace manifests. If desktop
work remains local, regenerate and verify the release lockfile from a clean
checkout without that workspace. Keep desktop files out of CLI release commits
and the Docker build context. Keep immutable installation enabled in CI and Docker.

The [CLI release workflow](../../.github/workflows/release-cli.yml) supports two entry points. A manual `workflow_dispatch` run builds the selected branch and uploads artifacts without creating a release. A `v*` tag push builds the checked-out tag. Its Python validation checks the Cargo workspace version format and, for tag runs, that the tag is `v` plus that version. Cargo's `--locked` release build checks that `Cargo.lock` is current. The CLI workflow does not depend on desktop manifests or sources; keep unrelated desktop changes out of CLI release commits. Enabled build targets are macOS 26 ARM64, macOS 26 Intel x86_64, and Ubuntu 26.04 GNU x86_64. Windows Server 2025 MSVC x86_64 is temporarily disabled because the real-process frontend proxy tests fail during child startup; restore its matrix entry and expected ZIP archive together. Each target runs `yarn workspace @sandpost/frontend build` before the Rust release binary build with `--locked`, then runs `cargo run --locked -p sandpost-test-runner -- smoke-test --binary "$BINARY_PATH" --frontend apps/frontend/dist` against that explicit release executable. The runner is verification tooling and is not included in release archives. On Windows, smoke cleanup force-terminates the server, so it does not verify graceful shutdown or drain behavior.

The CLI artifacts contain the executable and `LICENSE`, named with version, platform and architecture. Unix archives use `.tar.gz`, Windows uses `.zip`, and `SHA256SUMS` lists the downloadable artifacts. For a release, update manifests and lockfile, verify and commit the changes, push the commit, then create and push its annotated version tag (for example, `git tag -a v0.1.0-alpha.1 -m v0.1.0-alpha.1` followed by `git push origin v0.1.0-alpha.1`). A tag run waits for all target builds to succeed, then creates a GitHub draft release with the artifacts; prerelease versions are marked prerelease. Review and publish the draft manually. The workflow does not create tags, publish releases automatically, or build desktop installers. The existing CI workflow remains manually dispatched and unchanged.

Platform support is validated by successful release workflow builds and smoke tests; configuring a target alone does not establish compatibility. See [project build and release limits](project.md#build-and-run) for platform compatibility and signing status.

Each CLI matrix job restores `Swatinem/rust-cache` with a key derived from its Rust target and `Cargo.lock`, so unchanged third-party crates are reused between runs and each target keeps a separate cache. The container build caches third-party crates in a layer that depends only on the workspace manifests: `COPY --parents` copies `Cargo.toml`, `Cargo.lock`, crate manifests and build scripts, a placeholder build compiles the dependencies, and the real sources are copied afterwards. Changing application code rebuilds the workspace crates and reuses `tower`, `sqlx`, `tantivy` and the rest; editing any `Cargo.toml` or `Cargo.lock` invalidates that layer.

## Container publication

The root `Dockerfile` and `.dockerignore` are the container build entry points.
The [container release workflow](../../.github/workflows/release-container.yml)
builds Linux `amd64` and `arm64` musl images on native Ubuntu 26.04 runners.
Each architecture first builds the `verification` target, which runs the existing
real-binary smoke suite with embedded frontend verification, then starts and
restarts the final rootless `scratch` image with a read-only root filesystem and
a persistent named volume. Successful workflow runs establish compatibility;
the Docker build and image size have not yet been validated locally.

The image name follows the lowercase GitHub repository path, for example
`ghcr.io/north-foundry/sandpost`. Publishing uses the workflow's `GITHUB_TOKEN`
with `packages: write`; no registry secret is needed. After the first publication,
set the GHCR package visibility to public if users should be able to pull without
authentication. If the package already exists, ensure this repository has Actions
access to it.

A manual `workflow_dispatch` run builds and verifies both architectures without
logging into GHCR or publishing. Publishing a GitHub release triggers
verification of its checked-out version tag, builds and tests both architectures,
then publishes a multi-platform image tagged with the exact manifest version
without `v`, such as `ghcr.io/north-foundry/sandpost:0.1.0-alpha.1`. Only stable
versions also update `latest`. Draft releases and tag pushes alone do not publish images.
Each architecture is uploaded by digest after its checks; the public version tag
is assembled only after both builds pass. Container publication failure does not
unpublish an already published GitHub release.
