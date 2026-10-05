# 1.94 at least: the AWS SDK the object store adapter uses declares that as its
# minimum, and 1.91 refuses the whole workspace before compiling a line. A
# developer toolchain is newer than this image, so the mismatch only shows up
# here.
FROM rust:1.96-bookworm AS chef

WORKDIR /usr/local/src/autharie

RUN cargo install cargo-chef --version 0.1.77 --locked && \
    cargo install sqlx-cli --version 0.8.6 --locked --no-default-features --features postgres

# --- Plan: extract a recipe of the workspace dependency graph ----------
# Only the manifests and the lockfile shape this file, so a source-only
# change leaves the recipe -- and therefore the cook layer -- untouched.
FROM chef AS planner

COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# --- Cook: build every dependency from the recipe ----------------------
# Cached as long as the dependency graph is stable. This is the layer
# that used to be faked with dummy sources, and the reason the per-crate
# COPY list existed at all.
FROM chef AS builder

ENV SQLX_OFFLINE=true

COPY --from=planner /usr/local/src/autharie/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# --- Build the workspace binaries --------------------------------------
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim AS runtime

RUN \
    apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates=20230311+deb12u1 \
    libssl3=3.0.17-1~deb12u2 && \
    rm -rf /var/lib/apt/lists/* && \
    addgroup \
    --system \
    --gid 1000 \
    autharie && \
    adduser \
    --system \
    --no-create-home \
    --disabled-login \
    --uid 1000 \
    --gid 1000 \
    autharie

USER autharie

FROM rust:1.96-bookworm AS helm

ARG TARGETARCH
ARG HELM_VERSION=v3.17.3
ARG HELM_SHA256_AMD64=ee88b3c851ae6466a3de507f7be73fe94d54cbf2987cbaa3d1a3832ea331f2cd
ARG HELM_SHA256_ARM64=7944e3defd386c76fd92d9e6fec5c2d65a323f6fadc19bfb5e704e3eee10348e

RUN \
    case "${TARGETARCH}" in \
    amd64) sha="${HELM_SHA256_AMD64}" ;; \
    arm64) sha="${HELM_SHA256_ARM64}" ;; \
    *) echo "unsupported architecture: ${TARGETARCH}" >&2; exit 1 ;; \
    esac && \
    curl -fsSL -o /tmp/helm.tar.gz "https://get.helm.sh/helm-${HELM_VERSION}-linux-${TARGETARCH}.tar.gz" && \
    echo "${sha}  /tmp/helm.tar.gz" | sha256sum -c - && \
    tar -xzf /tmp/helm.tar.gz -C /tmp "linux-${TARGETARCH}/helm" && \
    install -m 0755 "/tmp/linux-${TARGETARCH}/helm" /usr/local/bin/helm

FROM runtime AS control-plane

COPY --from=helm /usr/local/bin/helm /usr/local/bin/helm
COPY charts/autharie-dataplane /usr/local/share/autharie/charts/autharie-dataplane
COPY --from=builder /usr/local/src/autharie/target/release/autharie-control-plane /usr/local/bin/
COPY --from=builder --chown=autharie:autharie /usr/local/src/autharie/libs/autharie-core/migrations /usr/local/src/autharie/migrations
COPY --from=builder /usr/local/cargo/bin/sqlx /usr/local/bin/

ENV CUSTOMER_CLOUD_CHART=/usr/local/share/autharie/charts/autharie-dataplane

EXPOSE 80

ENTRYPOINT [ "autharie-control-plane" ]

FROM runtime AS operator

COPY --from=builder /usr/local/src/autharie/target/release/autharie-operator /usr/local/bin/

EXPOSE 80

ENTRYPOINT [ "autharie-operator" ]

FROM runtime AS herald

COPY --from=builder /usr/local/src/autharie/target/release/herald /usr/local/bin/

ENTRYPOINT [ "herald" ]

FROM runtime AS genesis

COPY --from=builder /usr/local/src/autharie/target/release/genesis /usr/local/bin/

ENTRYPOINT [ "genesis" ]

FROM node:24.12-alpine AS console-build

WORKDIR /usr/local/src/autharie

ENV PNPM_HOME="/pnpm"
ENV PATH="$PNPM_HOME:$PATH"

RUN \
    corepack enable && \
    corepack prepare pnpm@9.15.0 --activate && \
    apk --no-cache add dumb-init=1.2.5-r3

COPY apps/console/package.json apps/console/pnpm-lock.yaml ./

RUN pnpm install --frozen-lockfile

COPY apps/console/ .

RUN pnpm run build

FROM nginx:1.28.0-alpine3.21-slim AS console

COPY --from=console-build /usr/local/src/autharie/dist /usr/local/src/autharie
COPY apps/console/nginx.conf /etc/nginx/conf.d/default.conf
COPY apps/console/docker-entrypoint.sh /docker-entrypoint.d/docker-entrypoint.sh

RUN chmod +x /docker-entrypoint.d/docker-entrypoint.sh
