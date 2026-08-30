FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive
ENV CHARIOX_SLICE_ROOT=/opt/chariox-slice

RUN apt-get update && apt-get install -y \
    ca-certificates \
    chromium \
    chromium-driver \
    chromium-sandbox \
    curl \
    git \
    jq \
    bubblewrap \
    build-essential \
    fonts-dejavu \
    fonts-liberation \
    novnc \
    openbox \
    pkg-config \
    procps \
    libdbus-1-dev \
    libssl-dev \
    protobuf-compiler \
    scrot \
    screen \
    socat \
    tesseract-ocr \
    tesseract-ocr-eng \
    lsof \
    unzip \
    websockify \
    x11-utils \
    x11vnc \
    xclip \
    xdotool \
    xvfb \
    zstd \
  && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
  && apt-get update \
  && apt-get install -y nodejs \
  && rm -rf /var/lib/apt/lists/*

RUN npm install -g @openai/codex opencode-ai @anthropic-ai/claude-code

RUN useradd -m -s /bin/bash slice
USER slice
WORKDIR /home/slice

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH="/home/slice/.cargo/bin:${PATH}"

USER root
RUN mkdir -p /opt/chariox-source/apps /opt/chariox-slice/bin /opt/chariox-slice/logs \
  && chown -R slice:slice /opt/chariox-source /opt/chariox-slice

COPY --chown=slice:slice apps/kernel /opt/chariox-source/apps/kernel
COPY --chown=slice:slice apps/relay /opt/chariox-source/apps/relay
COPY --chown=slice:slice examples/workflow-code /opt/chariox-source/examples/workflow-code

USER slice
RUN CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_TARGET_DIR=/tmp/chariox-kernel-target \
    cargo build --manifest-path /opt/chariox-source/apps/kernel/Cargo.toml --bin chariox-kernel \
  && cp /tmp/chariox-kernel-target/debug/chariox-kernel /opt/chariox-slice/bin/chariox-kernel \
  && rm -rf /tmp/chariox-kernel-target \
  && CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_TARGET_DIR=/tmp/chariox-relay-target \
    cargo build --manifest-path /opt/chariox-source/apps/relay/Cargo.toml --bin chariox-relay \
  && cp /tmp/chariox-relay-target/debug/chariox-relay /opt/chariox-slice/bin/chariox-relay \
  && rm -rf /tmp/chariox-relay-target /home/slice/.cargo/registry /home/slice/.cargo/git

COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-cdp.mjs /opt/chariox-slice/browser-cdp.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-cdp.mjs /opt/chariox-slice/browser-controller-cdp.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-snapshot.mjs /opt/chariox-slice/browser-controller-snapshot.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller.mjs /opt/chariox-slice/browser-controller.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/provider-port-bridge.mjs /opt/chariox-slice/provider-port-bridge.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/slice-screen.sh /opt/chariox-slice/slice-screen.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/start-providers.sh /opt/chariox-slice/start-providers.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/start-runtime.sh /opt/chariox-slice/start-runtime.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/validate-screen.sh /opt/chariox-slice/validate-screen.sh

RUN chmod +x /opt/chariox-slice/browser-cdp.mjs /opt/chariox-slice/browser-controller-cdp.mjs /opt/chariox-slice/browser-controller-snapshot.mjs /opt/chariox-slice/browser-controller.mjs /opt/chariox-slice/provider-port-bridge.mjs /opt/chariox-slice/slice-screen.sh /opt/chariox-slice/start-providers.sh /opt/chariox-slice/start-runtime.sh /opt/chariox-slice/validate-screen.sh

USER root
RUN apt-get update \
  && apt-get install -y gh \
  && rm -rf /var/lib/apt/lists/*
USER slice

ARG CHARIOX_RELAY_PEER_PROTOCOL_VERSION=unknown
ARG CHARIOX_RUNTIME_SOURCE_REVISION=unknown
LABEL io.chariox.relay-peer-protocol-version="${CHARIOX_RELAY_PEER_PROTOCOL_VERSION}"
LABEL io.chariox.runtime-source-revision="${CHARIOX_RUNTIME_SOURCE_REVISION}"

RUN printf '%s\n' "${CHARIOX_RUNTIME_SOURCE_REVISION}" > /opt/chariox-slice/runtime-source-revision

EXPOSE 43119 43130 43140 43252 6080
WORKDIR /workspace
CMD ["sleep", "infinity"]
