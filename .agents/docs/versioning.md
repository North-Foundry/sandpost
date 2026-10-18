# Versioning and releases

Sandpost uses one shared version for its Rust crates, CLI/runtime and desktop app. Set the Rust version in `[workspace.package]` in the root `Cargo.toml`; Rust crates inherit it. Keep `apps/desktop/package.json` at the same version. Private packages used only for unrelated tooling do not need to share it. `Cargo.lock` must reflect the release manifests and be committed with them. The initial approved release version is `0.1.0-alpha.1`.

Use numeric prerelease identifiers and progress in this order, resetting the number at each stage: `0.1.0-alpha.1`, `0.1.0-alpha.2`, `0.1.0-beta.1`, `0.1.0-rc.1`, then stable `0.1.0`. Subsequent prereleases increment their stage number; stable versions follow semantic versioning. The Git tag is `v` plus the exact manifest version, such as `v0.1.0-alpha.1`; the `v` belongs only in the tag. Never move or recreate a published tag. Fix a published release by making a new version and tag.

The CLI release lockfile must match the committed workspace manifests. If desktop
work remains local, regenerate and verify the release lockfile from a clean
checkout without that workspace. Keep desktop files out of CLI release commits
and the Docker build context. Keep immutable installation enabled in CI and Docker.

The [CLI release workflow](../../.github/workflows/release-cli.yml) supports two entry points. A manual `workflow_dispatch` run builds the selected branch and uploads artifacts without creating a release. A `v*` tag push builds the checked-out tag. Its Python validation checks the Cargo workspace version format and, for tag runs, that the tag is `v` plus that version. Cargo's `--locked` release build checks that `Cargo.lock` is current. The CLI workflow does not depend on desktop manifests or sources; keep unrelated desktop changes out of CLI release commits. Enabled build targets are macOS 26 ARM64, macOS 26 Intel x86_64, and Ubuntu 26.04 GNU x86_64. Windows Server 2025 MSVC x86_64 is temporarily disabled because the real-process frontend proxy tests fail during child startup; restore its matrix entry and expected ZIP archive together. Each target runs `yarn workspace @sandpost/frontend build` before the Rust release binary build with `--locked`, then runs `cargo run --locked -p sandpost-test-runner -- smoke-test --binary "$BINARY_PATH" --frontend apps/frontend/dist` against that explicit release executable. The runner is verification tooling and is not included in release archives. On Windows, smoke cleanup force-terminates the server, so it does not verify graceful shutdown or drain behavior.

The CLI artifacts contain the executable and `LICENSE`, named with version, platform and architecture. Unix archives use `.tar.gz`, Windows uses `.zip`, and `SHA256SUMS` lists the downloadable artifacts. For a release, update manifests and lockfile, verify and commit the changes, push the commit, then create and push its annotated version tag (for example, `git tag -a v0.1.0-alpha.1 -m v0.1.0-alpha.1` followed by `git push origin v0.1.0-alpha.1`). A tag run waits for all target builds to succeed, then creates a GitHub draft release with the artifacts; prerelease versions are marked prerelease. Review and publish the draft manually. The workflow does not create tags, publish releases automatically, or build desktop installers. The existing CI workflow remains manually dispatched and unchanged.

Platform support is validated by successful release workflow builds and smoke tests; configuring a target alone does not establish compatibility. See [project build and release limits](project.md#build-and-run) for platform compatibility and signing status.

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
