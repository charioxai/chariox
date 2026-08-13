# syntax=docker/dockerfile:1

FROM node:22-bookworm-slim AS cli-builder
WORKDIR /opt/chariox
RUN corepack enable && corepack prepare pnpm@9.15.0 --activate
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY apps/cli/package.json apps/cli/package.json
COPY packages packages
RUN pnpm install --frozen-lockfile
COPY apps/cli apps/cli
RUN pnpm --filter @chariox/cli run build

FROM rust:1.88-bookworm AS rust-builder
WORKDIR /opt/chariox
COPY apps/relay apps/relay
COPY apps/kernel apps/kernel
RUN cargo build --manifest-path apps/kernel/Cargo.toml --release --bin chariox-kernel --bin chariox-cli \
  && cargo build --manifest-path apps/relay/Cargo.toml --release --bin chariox-relay

FROM node:22-bookworm-slim
LABEL org.opencontainers.image.title="Chariox remote-machine lab"
LABEL org.opencontainers.image.description="Chariox CLI, daemon, and relay runtime image for isolated remote-machine/provider-account testing."

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
  && apt-get install -y --no-install-recommends \
    bash \
    ca-certificates \
    curl \
    git \
    jq \
    openssh-client \
    procps \
    ripgrep \
    tini \
    unzip \
    xdg-utils \
    zsh \
  && rm -rf /var/lib/apt/lists/*

RUN corepack enable && corepack prepare pnpm@9.15.0 --activate
RUN curl -fsSL https://bun.sh/install | bash \
  && cp /root/.bun/bin/bun /usr/local/bin/bun \
  && chmod +x /usr/local/bin/bun
RUN useradd --create-home --shell /usr/bin/zsh chariox

WORKDIR /opt/chariox
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY --from=cli-builder /opt/chariox/node_modules ./node_modules
COPY --from=cli-builder /opt/chariox/apps/cli ./apps/cli
COPY --from=cli-builder /opt/chariox/packages ./packages
COPY --from=rust-builder /opt/chariox/apps/kernel/target/release/chariox-kernel /usr/local/bin/chariox-kernel
COPY --from=rust-builder /opt/chariox/apps/kernel/target/release/chariox-cli /usr/local/bin/chariox-cli
COPY --from=rust-builder /opt/chariox/apps/relay/target/release/chariox-relay /usr/local/bin/chariox-relay
COPY docker/lab/entrypoint.sh /usr/local/bin/chariox-container
COPY docker/lab/chariox-open-url /usr/local/bin/chariox-open-url
RUN mkdir -p /opt/chariox/apps/kernel \
  && chmod +x /usr/local/bin/chariox-container /usr/local/bin/chariox-open-url \
  && ln -sf /usr/local/bin/chariox-cli /usr/local/bin/chariox \
  && ln -sf /usr/local/bin/chariox-open-url /usr/local/bin/xdg-open \
  && chown -R chariox:chariox /opt/chariox /home/chariox

USER chariox
WORKDIR /home/chariox/workspace
ENV CHARIOX_CONFIG_DIR=/home/chariox/.config/chariox
ENV CHARIOX_DATA_DIR=/home/chariox/.local/share/chariox
ENV CHARIOX_RUNTIME_DIR=/home/chariox/.cache/chariox/runtime
ENV CHARIOX_SESSION_HISTORY_DIR=/home/chariox/.local/share/chariox/sessions
ENV BROWSER=/usr/local/bin/chariox-open-url
ENV BUN_BIN=/usr/local/bin/bun
ENV NPM_CONFIG_PREFIX=/home/chariox/.local
ENV PATH=/home/chariox/.local/bin:/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin

EXPOSE 43118 43120 43150
ENTRYPOINT ["tini", "--", "chariox-container"]
CMD ["daemon"]
