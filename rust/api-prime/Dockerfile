# syntax=docker/dockerfile:1.7

# Multi-arch Dockerfile for the smooai-api-prime workspace crate.
#
# This single Dockerfile builds either of the two binaries that ship from
# the crate, selected by `--build-arg BINARY=<name>`:
#
#   - api-prime             (data plane, default)
#   - api-prime-controller  (control plane)
#
# Build context note:
#   The workspace at `rust/` shares one Cargo.lock with sibling members
#   (observability, api-prime). Pass `context: ./rust` and
#   `file: ./rust/api-prime/Dockerfile`.
#
# Usage:
#   docker buildx build \
#     --platform linux/arm64 \
#     --build-arg BINARY=api-prime \
#     -t ghcr.io/smooai/api-prime:dev \
#     -f rust/api-prime/Dockerfile \
#     rust/

ARG RUST_VERSION=1.91
ARG BINARY=api-prime

############################
# Stage 1 — build
############################
FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-slim-bookworm AS builder

ARG TARGETARCH
ARG BINARY

RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates \
        git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

# Copy the full rust workspace. Build context is rust/, so members
# (observability, api-prime) live as siblings.
COPY . .

# Build only the requested binary in release mode. Cache cargo's index
# and target dir via BuildKit mounts to keep rebuilds fast.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/workspace/target \
    cargo build --release --locked -p smooai-api-prime --bin "${BINARY}" \
    && cp "/workspace/target/release/${BINARY}" "/usr/local/bin/${BINARY}"

############################
# Stage 2 — minimal runtime (distroless cc)
############################
# We use distroless cc rather than distroless static because tokio +
# sqlx + reqwest link against the glibc runtime even when TLS is rustls.
# Distroless cc ships glibc + ca-certificates and runs as nonroot:65532.
FROM gcr.io/distroless/cc-debian12:nonroot AS runtime

ARG BINARY

# Bake the binary path into a known location. Distroless has no shell,
# so we can't templatize ENTRYPOINT with env vars — we COPY into a
# stable path and use that as the entrypoint.
COPY --from=builder /usr/local/bin/${BINARY} /usr/local/bin/app

# Document both ports. Data plane historically used 3000; controller
# defaults to 8080. The workload spec (k8s) selects the actual one.
EXPOSE 3000
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/app"]
