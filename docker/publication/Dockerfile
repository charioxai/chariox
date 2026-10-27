# syntax=docker/dockerfile:1

FROM node:22.17.1-bookworm@sha256:37ff334612f77d8f999c10af8797727b731629c26f2e83caa6af390998bdc49c AS ca-bundle
RUN echo 'a3413a37a8e09cc21b2c11c9ffb23d92d2fc9d1933c9e7617f5c4fba4f72d37d  /etc/ssl/certs/ca-certificates.crt' | sha256sum --check --strict

FROM node:22-bookworm-slim@sha256:d649c27dae7ba0137b3cef5dd75baa422c08dc3d9e3fc0c23dfb172dc3cc6436 AS js-toolchain
WORKDIR /opt/chariox-toolchain
COPY docker/publication/toolchain/package.json docker/publication/toolchain/package-lock.json ./
COPY deploy/managed-kernel/provider-versions.env ./
COPY docker/publication/toolchain/canonicalize-sbom.mjs ./
RUN . ./provider-versions.env \
  && npm ci --omit=dev \
  && npm sbom --sbom-format cyclonedx | node canonicalize-sbom.mjs > toolchain-sbom.cdx.json \
  && test "$(node_modules/.bin/pnpm --version)" = "9.15.0" \
  && test "$(node_modules/.bin/codex --version)" = "codex-cli ${CHARIOX_CODEX_VERSION}" \
  && test "$(node_modules/.bin/opencode --version)" = "${CHARIOX_OPENCODE_VERSION}" \
  && test "$(node_modules/.bin/claude --version)" = "${CHARIOX_CLAUDE_VERSION} (Claude Code)"

FROM js-toolchain AS js-builder
ENV PATH=/opt/chariox-toolchain/node_modules/.bin:/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin
WORKDIR /opt/chariox
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY patches patches
COPY packages packages
COPY apps/server/package.json apps/server/package.json
RUN pnpm install --frozen-lockfile
COPY apps/server apps/server
# The server build runs the protocol pin check, which reads these files.
COPY scripts/generate-protocol-versions.mjs scripts/generate-protocol-versions.mjs
COPY scripts/lib/protocol-versions.mjs scripts/lib/protocol-versions.mjs
COPY fixtures/protocol-versions.json fixtures/historical-workflow-protocol.json fixtures/
COPY apps/kernel/managed-upgrade-protocol-transitions.json apps/kernel/managed-upgrade-protocol-transitions.json
COPY apps/kernel/src/local/api/types.rs apps/kernel/src/local/api/types.rs
COPY apps/kernel/src/transport/relay_peer.rs apps/kernel/src/transport/relay_peer.rs
COPY docker/publication/Dockerfile docker/publication/Dockerfile
RUN pnpm --filter @chariox/tool-display run build \
  && pnpm --filter @chariox/server run build

FROM rust:1.88-bookworm@sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0 AS rust-builder
ARG CARGO_BUILD_JOBS=1
ARG CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION=629
