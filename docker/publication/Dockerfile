# syntax=docker/dockerfile:1

FROM node:22-bookworm-slim AS js-builder
WORKDIR /opt/arroba
RUN corepack enable && corepack prepare pnpm@9.15.0 --activate
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY packages packages
COPY apps/server/package.json apps/server/package.json
RUN pnpm install --frozen-lockfile
COPY apps/server apps/server
RUN pnpm --filter @arroba/tool-display run build \
  && pnpm --filter @arroba/server run build

FROM rust:1.88-bookworm AS rust-builder
ARG ARROBA_LOCAL_DAEMON_PROTOCOL_VERSION=243
WORKDIR /opt/arroba
RUN apt-get update \
  && apt-get install -y --no-install-recommends \
    libdbus-1-dev \
    pkg-config \
  && rm -rf /var/lib/apt/lists/*
COPY apps/kernel apps/kernel
COPY apps/relay apps/relay
COPY examples/workflow-code examples/workflow-code
RUN cargo build --manifest-path apps/kernel/Cargo.toml --release --bin arroba-kernel \
  && test "$(apps/kernel/target/release/arroba-kernel --print-local-daemon-protocol-version)" = "${ARROBA_LOCAL_DAEMON_PROTOCOL_VERSION}"

FROM node:22-bookworm-slim
LABEL org.opencontainers.image.title="Arroba workflow publication"
LABEL org.opencontainers.image.description="Standalone Arroba kernel plus workflow publication gateway runtime."

ARG ARROBA_CODEX_VERSION=0.144.0
ARG ARROBA_OPENCODE_VERSION=1.4.1
ARG ARROBA_CLAUDE_VERSION=2.1.207
ARG ARROBA_LOCAL_DAEMON_PROTOCOL_VERSION=243
LABEL dev.arroba.provider.codex.version="${ARROBA_CODEX_VERSION}"
LABEL dev.arroba.provider.opencode.version="${ARROBA_OPENCODE_VERSION}"
LABEL dev.arroba.provider.claude.version="${ARROBA_CLAUDE_VERSION}"
LABEL dev.arroba.local-daemon-protocol-version="${ARROBA_LOCAL_DAEMON_PROTOCOL_VERSION}"

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
  && apt-get install -y --no-install-recommends \
    bash \
    ca-certificates \
    libdbus-1-3 \
    tini \
  && rm -rf /var/lib/apt/lists/*

RUN npm install -g \
    "@openai/codex@${ARROBA_CODEX_VERSION}" \
    "opencode-ai@${ARROBA_OPENCODE_VERSION}" \
    "@anthropic-ai/claude-code@${ARROBA_CLAUDE_VERSION}" \
  && test "$(codex --version)" = "codex-cli ${ARROBA_CODEX_VERSION}" \
  && test "$(opencode --version)" = "${ARROBA_OPENCODE_VERSION}" \
  && test "$(claude --version)" = "${ARROBA_CLAUDE_VERSION} (Claude Code)"

RUN useradd --create-home --uid 1001 --shell /bin/bash arroba \
  && useradd --create-home --uid 1002 --shell /usr/sbin/nologin arroba-action \
  && useradd --create-home --uid 1003 --shell /usr/sbin/nologin arroba-gateway

WORKDIR /opt/arroba
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY --from=js-builder /opt/arroba/node_modules ./node_modules
COPY --from=js-builder /opt/arroba/packages ./packages
COPY --from=js-builder /opt/arroba/apps/server ./apps/server
COPY --from=rust-builder /opt/arroba/apps/kernel/target/release/arroba-kernel /usr/local/bin/arroba-kernel
COPY docker/publication/entrypoint.sh /usr/local/bin/arroba-publication-container
COPY docker/publication/wait-for-tcp.mjs /usr/local/bin/arroba-wait-for-tcp.mjs
RUN chmod +x /usr/local/bin/arroba-publication-container /usr/local/bin/arroba-wait-for-tcp.mjs \
  && chown -R root:root /opt/arroba \
  && chmod -R go-w /opt/arroba \
  && chown -R arroba:arroba /home/arroba \
  && chown -R arroba-action:arroba-action /home/arroba-action \
  && chown -R arroba-gateway:arroba-gateway /home/arroba-gateway \
  && chmod 700 /home/arroba /home/arroba-action /home/arroba-gateway

WORKDIR /workspace
ENV HOME=/home/arroba
ENV ARROBA_CONFIG_DIR=/home/arroba/.config/arroba
ENV ARROBA_DATA_DIR=/home/arroba/.local/share/arroba
ENV ARROBA_RUNTIME_DIR=/home/arroba/.cache/arroba/runtime
ENV ARROBA_SESSION_HISTORY_DIR=/home/arroba/.local/share/arroba/sessions
ENV ARROBA_KERNEL_HOST=127.0.0.1
ENV ARROBA_KERNEL_PORT=43118
ENV HOST=0.0.0.0
ENV PORT=3000
ENV PATH=/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin

EXPOSE 3000 43118
ENTRYPOINT ["tini", "--", "arroba-publication-container"]
CMD ["standalone"]
