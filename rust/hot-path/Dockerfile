# syntax=docker/dockerfile:1.7

# Multi-arch Dockerfile for smooai-hot-path.
#
# Build a workspace-aware image. We need the surrounding rust/ workspace
# Cargo.toml + the sibling crates referenced by `[workspace] members`, so
# the build context for this Dockerfile is `rust/` (one level up).
#
# Usage:
#   docker buildx build \
#     --platform linux/arm64 \
#     -t ghcr.io/smooai/hot-path:dev \
#     -f rust/hot-path/Dockerfile \
#     rust/

ARG RUST_VERSION=1.90

############################
# Stage 1 — cargo-chef plan + build
############################
FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-slim-bookworm AS builder

ARG TARGETARCH

RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates \
        git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

# Copy the full rust workspace. Build context is rust/, so members
# (observability, log-viewer, hot-path) live as siblings.
COPY . .

# Build only the hot-path binary in release mode. Cache cargo's index
# and target dir via BuildKit mounts to keep rebuilds fast.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/workspace/target \
    cargo build --release --locked -p smooai-hot-path \
    && cp /workspace/target/release/smooai-hot-path /usr/local/bin/smooai-hot-path

############################
# Stage 2 — minimal runtime (distroless cc)
############################
# We use distroless cc rather than distroless static because tokio +
# sqlx + reqwest link against the glibc runtime even when TLS is rustls.
# Distroless cc ships glibc + ca-certificates and runs as nonroot:65532.
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

COPY --from=builder /usr/local/bin/smooai-hot-path /usr/local/bin/smooai-hot-path

EXPOSE 3000
ENV PORT=3000

ENTRYPOINT ["/usr/local/bin/smooai-hot-path"]
