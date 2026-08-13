# syntax=docker/dockerfile:1

FROM node:22-bookworm-slim AS js-builder
WORKDIR /opt/chariox
RUN corepack enable && corepack prepare pnpm@9.15.0 --activate
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY packages packages
COPY apps/server/package.json apps/server/package.json
RUN pnpm install --frozen-lockfile
COPY apps/server apps/server
RUN pnpm --filter @chariox/tool-display run build \
  && pnpm --filter @chariox/server run build

FROM rust:1.88-bookworm AS rust-builder
ARG CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION=254
WORKDIR /opt/chariox
RUN apt-get update \
  && apt-get install -y --no-install-recommends \
    libdbus-1-dev \
    pkg-config \
  && rm -rf /var/lib/apt/lists/*
COPY apps/kernel apps/kernel
COPY apps/relay apps/relay
COPY examples/workflow-code examples/workflow-code
RUN cargo build --manifest-path apps/kernel/Cargo.toml --release --bin chariox-kernel \
  && test "$(apps/kernel/target/release/chariox-kernel --print-local-daemon-protocol-version)" = "${CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION}"

FROM node:22-bookworm-slim
LABEL org.opencontainers.image.title="Chariox workflow publication"
LABEL org.opencontainers.image.description="Standalone Chariox kernel plus workflow publication gateway runtime."

ARG CHARIOX_CODEX_VERSION=0.144.0
ARG CHARIOX_OPENCODE_VERSION=1.4.1
ARG CHARIOX_CLAUDE_VERSION=2.1.207
ARG CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION=254
LABEL dev.chariox.provider.codex.version="${CHARIOX_CODEX_VERSION}"
LABEL dev.chariox.provider.opencode.version="${CHARIOX_OPENCODE_VERSION}"
LABEL dev.chariox.provider.claude.version="${CHARIOX_CLAUDE_VERSION}"
LABEL dev.chariox.local-daemon-protocol-version="${CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION}"

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
  && apt-get install -y --no-install-recommends \
    bash \
    ca-certificates \
    libdbus-1-3 \
    tini \
  && rm -rf /var/lib/apt/lists/*

RUN npm install -g \
    "@openai/codex@${CHARIOX_CODEX_VERSION}" \
    "opencode-ai@${CHARIOX_OPENCODE_VERSION}" \
    "@anthropic-ai/claude-code@${CHARIOX_CLAUDE_VERSION}" \
  && test "$(codex --version)" = "codex-cli ${CHARIOX_CODEX_VERSION}" \
  && test "$(opencode --version)" = "${CHARIOX_OPENCODE_VERSION}" \
  && test "$(claude --version)" = "${CHARIOX_CLAUDE_VERSION} (Claude Code)"

RUN useradd --create-home --uid 1001 --shell /bin/bash chariox \
  && useradd --create-home --uid 1002 --shell /usr/sbin/nologin chariox-action \
  && useradd --create-home --uid 1003 --shell /usr/sbin/nologin chariox-gateway

WORKDIR /opt/chariox
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY --from=js-builder /opt/chariox/node_modules ./node_modules
COPY --from=js-builder /opt/chariox/packages ./packages
COPY --from=js-builder /opt/chariox/apps/server ./apps/server
COPY --from=rust-builder /opt/chariox/apps/kernel/target/release/chariox-kernel /usr/local/bin/chariox-kernel
COPY docker/publication/entrypoint.sh /usr/local/bin/chariox-publication-container
COPY docker/publication/wait-for-tcp.mjs /usr/local/bin/chariox-wait-for-tcp.mjs
RUN chmod +x /usr/local/bin/chariox-publication-container /usr/local/bin/chariox-wait-for-tcp.mjs \
  && chown -R root:root /opt/chariox \
  && chmod -R go-w /opt/chariox \
  && chown -R chariox:chariox /home/chariox \
  && chown -R chariox-action:chariox-action /home/chariox-action \
  && chown -R chariox-gateway:chariox-gateway /home/chariox-gateway \
  && chmod 700 /home/chariox /home/chariox-action /home/chariox-gateway

WORKDIR /workspace
ENV HOME=/home/chariox
ENV CHARIOX_CONFIG_DIR=/home/chariox/.config/chariox
ENV CHARIOX_DATA_DIR=/home/chariox/.local/share/chariox
ENV CHARIOX_RUNTIME_DIR=/home/chariox/.cache/chariox/runtime
ENV CHARIOX_SESSION_HISTORY_DIR=/home/chariox/.local/share/chariox/sessions
ENV CHARIOX_KERNEL_HOST=127.0.0.1
ENV CHARIOX_KERNEL_PORT=43118
ENV HOST=0.0.0.0
ENV PORT=3000
ENV PATH=/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin

EXPOSE 3000 43118
ENTRYPOINT ["tini", "--", "chariox-publication-container"]
CMD ["standalone"]
