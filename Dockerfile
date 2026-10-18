# syntax=docker/dockerfile:1
FROM node:24.22-alpine3.24 AS frontend

WORKDIR /work
ENV YARN_ENABLE_IMMUTABLE_INSTALLS=true
RUN corepack enable
COPY package.json yarn.lock .yarnrc.yml ./
COPY apps/ apps/
COPY packages/ packages/
COPY crates/sandpost/package.json crates/sandpost/package.json
RUN --mount=type=cache,target=/root/.yarn/berry/cache \
    yarn workspaces focus @sandpost/frontend && \
    yarn workspace @sandpost/frontend build

FROM rust:1.99-alpine3.24 AS rust-build

ARG TARGETARCH
RUN apk add --no-cache build-base cmake perl ca-certificates
WORKDIR /work
ENV CARGO_TARGET_DIR=/target
ENV RUSTFLAGS="-C target-feature=+crt-static"
COPY Cargo.toml Cargo.lock ./
COPY LICENSE /LICENSE
COPY crates/ crates/
COPY --from=frontend /work/apps/frontend/dist apps/frontend/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,id=rust-target-${TARGETARCH},target=/target \
    case "$TARGETARCH" in \
      amd64) rust_target=x86_64-unknown-linux-musl ;; \
      arm64) rust_target=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported build architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac && \
    rustup target add "$rust_target" && \
    cargo build --release --locked -p sandpost --target "$rust_target" && \
    mkdir -p /out && \
    cp "/target/$rust_target/release/sandpost" /out/sandpost && \
    if readelf -l /out/sandpost | grep -q INTERP || \
       readelf -d /out/sandpost | grep -q '(NEEDED)'; then \
      echo 'runtime binary must be statically linked' >&2; exit 1; \
    fi && \
    printf '%s' "$rust_target" > /out/rust-target && \
    mkdir -p /runtime-data /runtime-tmp && \
    chown 65532:65532 /runtime-data /runtime-tmp

FROM rust-build AS verification

ARG TARGETARCH

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,id=rust-target-${TARGETARCH},target=/target \
    rust_target="$(cat /out/rust-target)" && \
    cargo run --release --locked -p sandpost-test-runner --target "$rust_target" -- \
      smoke-test --binary /out/sandpost --frontend apps/frontend/dist

FROM scratch

LABEL org.opencontainers.image.title="Sandpost" \
    org.opencontainers.image.description="Self-hosted development email catcher" \
    org.opencontainers.image.source="https://github.com/North-Foundry/sandpost" \
    org.opencontainers.image.licenses="MIT"

COPY --from=rust-build /out/sandpost /sandpost
COPY --from=rust-build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=rust-build /LICENSE /LICENSE
COPY --from=rust-build --chown=65532:65532 /runtime-data /data
COPY --from=rust-build --chown=65532:65532 /runtime-tmp /tmp

USER 65532:65532
ENV HOME=/data \
    SANDPOST_DATA_DIRECTORY=/data \
    SANDPOST_HTTP_BIND_ADDRESS=0.0.0.0 \
    SANDPOST_HTTP_PORT=8025 \
    SANDPOST_SMTP_BIND_ADDRESS=0.0.0.0 \
    SANDPOST_SMTP_PORT=1025 \
    SANDPOST_IMAP_ENABLED=false

EXPOSE 8025 1025
ENTRYPOINT ["/sandpost"]
