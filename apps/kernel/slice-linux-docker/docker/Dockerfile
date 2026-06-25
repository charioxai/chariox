FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive
ENV ARROBA_SLICE_ROOT=/opt/arroba-slice

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

RUN npm install -g @openai/codex opencode-ai @anthropic-ai/claude-code \
  && npm install -g --ignore-scripts @earendil-works/pi-coding-agent

RUN useradd -m -s /bin/bash slice
USER slice
WORKDIR /home/slice

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH="/home/slice/.cargo/bin:${PATH}"

USER root
RUN mkdir -p /opt/arroba-source/apps /opt/arroba-source/examples /opt/arroba-slice/bin /opt/arroba-slice/logs \
  && chown -R slice:slice /opt/arroba-source /opt/arroba-slice

COPY --chown=slice:slice apps/kernel /opt/arroba-source/apps/kernel
COPY --chown=slice:slice apps/relay /opt/arroba-source/apps/relay
COPY --chown=slice:slice examples/workflow-code /opt/arroba-source/examples/workflow-code

USER slice
RUN CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_TARGET_DIR=/tmp/arroba-kernel-target \
    cargo build --manifest-path /opt/arroba-source/apps/kernel/Cargo.toml --bin arroba-kernel \
  && cp /tmp/arroba-kernel-target/debug/arroba-kernel /opt/arroba-slice/bin/arroba-kernel \
  && rm -rf /tmp/arroba-kernel-target \
  && CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_TARGET_DIR=/tmp/arroba-relay-target \
    cargo build --manifest-path /opt/arroba-source/apps/relay/Cargo.toml --bin arroba-relay \
  && cp /tmp/arroba-relay-target/debug/arroba-relay /opt/arroba-slice/bin/arroba-relay \
  && rm -rf /tmp/arroba-relay-target /home/slice/.cargo/registry /home/slice/.cargo/git

COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-cdp.mjs /opt/arroba-slice/browser-cdp.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/slice-screen.sh /opt/arroba-slice/slice-screen.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/start-providers.sh /opt/arroba-slice/start-providers.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/start-runtime.sh /opt/arroba-slice/start-runtime.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/validate-screen.sh /opt/arroba-slice/validate-screen.sh

RUN chmod +x /opt/arroba-slice/browser-cdp.mjs /opt/arroba-slice/slice-screen.sh /opt/arroba-slice/start-providers.sh /opt/arroba-slice/start-runtime.sh /opt/arroba-slice/validate-screen.sh

EXPOSE 43119 43130 43140 43252 6080
WORKDIR /workspace
CMD ["sleep", "infinity"]
