# syntax=docker/dockerfile:1

FROM node:22-bookworm-slim AS js-builder
WORKDIR /opt/arroba
RUN corepack enable && corepack prepare pnpm@9.15.0 --activate
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY packages packages
COPY apps/server/package.json apps/server/package.json
RUN pnpm install --frozen-lockfile
COPY apps/server apps/server
RUN pnpm --filter @arroba/server run build

FROM rust:1.88-bookworm AS rust-builder
WORKDIR /opt/arroba
RUN apt-get update \
  && apt-get install -y --no-install-recommends \
    libdbus-1-dev \
    pkg-config \
  && rm -rf /var/lib/apt/lists/*
COPY apps/kernel apps/kernel
COPY apps/relay apps/relay
RUN cargo build --manifest-path apps/kernel/Cargo.toml --release --bin arroba-kernel

FROM node:22-bookworm-slim
LABEL org.opencontainers.image.title="Arroba workflow publication"
LABEL org.opencontainers.image.description="Standalone Arroba kernel plus workflow publication gateway runtime."

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
  && apt-get install -y --no-install-recommends \
    bash \
    ca-certificates \
    libdbus-1-3 \
    tini \
  && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --shell /bin/bash arroba

WORKDIR /opt/arroba
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY --from=js-builder /opt/arroba/node_modules ./node_modules
COPY --from=js-builder /opt/arroba/packages ./packages
COPY --from=js-builder /opt/arroba/apps/server ./apps/server
COPY --from=rust-builder /opt/arroba/apps/kernel/target/release/arroba-kernel /usr/local/bin/arroba-kernel
COPY docker/publication/entrypoint.sh /usr/local/bin/arroba-publication-container
COPY docker/publication/wait-for-tcp.mjs /usr/local/bin/arroba-wait-for-tcp.mjs
RUN chmod +x /usr/local/bin/arroba-publication-container /usr/local/bin/arroba-wait-for-tcp.mjs \
  && chown -R arroba:arroba /opt/arroba /home/arroba

USER arroba
WORKDIR /workspace
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
