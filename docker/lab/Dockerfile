# syntax=docker/dockerfile:1

FROM node:22-bookworm-slim AS cli-builder
WORKDIR /opt/arroba
RUN corepack enable && corepack prepare pnpm@9.15.0 --activate
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY apps/cli/package.json apps/cli/package.json
COPY packages packages
RUN pnpm install --frozen-lockfile
COPY apps/cli apps/cli
RUN pnpm --filter @arroba/cli run build

FROM rust:1.88-bookworm AS rust-builder
WORKDIR /opt/arroba
COPY apps/relay apps/relay
COPY apps/kernel apps/kernel
RUN cargo build --manifest-path apps/kernel/Cargo.toml --release --bin arroba-kernel --bin arroba-cli \
  && cargo build --manifest-path apps/relay/Cargo.toml --release --bin arroba-relay

FROM node:22-bookworm-slim
LABEL org.opencontainers.image.title="Arroba remote-machine lab"
LABEL org.opencontainers.image.description="Arroba CLI, daemon, and relay runtime image for isolated remote-machine/provider-account testing."

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
RUN useradd --create-home --shell /usr/bin/zsh arroba

WORKDIR /opt/arroba
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY --from=cli-builder /opt/arroba/node_modules ./node_modules
COPY --from=cli-builder /opt/arroba/apps/cli ./apps/cli
COPY --from=cli-builder /opt/arroba/packages ./packages
COPY --from=rust-builder /opt/arroba/apps/kernel/target/release/arroba-kernel /usr/local/bin/arroba-kernel
COPY --from=rust-builder /opt/arroba/apps/kernel/target/release/arroba-cli /usr/local/bin/arroba-cli
COPY --from=rust-builder /opt/arroba/apps/relay/target/release/arroba-relay /usr/local/bin/arroba-relay
COPY docker/lab/entrypoint.sh /usr/local/bin/arroba-container
COPY docker/lab/arroba-open-url /usr/local/bin/arroba-open-url
RUN mkdir -p /opt/arroba/apps/kernel \
  && chmod +x /usr/local/bin/arroba-container /usr/local/bin/arroba-open-url \
  && ln -sf /usr/local/bin/arroba-cli /usr/local/bin/arroba \
  && ln -sf /usr/local/bin/arroba-open-url /usr/local/bin/xdg-open \
  && chown -R arroba:arroba /opt/arroba /home/arroba

USER arroba
WORKDIR /home/arroba/workspace
ENV ARROBA_CONFIG_DIR=/home/arroba/.config/arroba
ENV ARROBA_DATA_DIR=/home/arroba/.local/share/arroba
ENV ARROBA_RUNTIME_DIR=/home/arroba/.cache/arroba/runtime
ENV ARROBA_SESSION_HISTORY_DIR=/home/arroba/.local/share/arroba/sessions
ENV BROWSER=/usr/local/bin/arroba-open-url
ENV BUN_BIN=/usr/local/bin/bun
ENV NPM_CONFIG_PREFIX=/home/arroba/.local
ENV PATH=/home/arroba/.local/bin:/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin

EXPOSE 43118 43120 43150
ENTRYPOINT ["tini", "--", "arroba-container"]
CMD ["daemon"]
