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
  && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
  && apt-get update \
  && apt-get install -y nodejs \
  && rm -rf /var/lib/apt/lists/*

RUN npm install -g @openai/codex opencode-ai

RUN useradd -m -s /bin/bash slice
USER slice
WORKDIR /home/slice

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH="/home/slice/.cargo/bin:${PATH}"

USER root
RUN mkdir -p /opt/arroba-source/apps /opt/arroba-slice/bin /opt/arroba-slice/logs \
  && chown -R slice:slice /opt/arroba-source /opt/arroba-slice

COPY --chown=slice:slice apps/kernel /opt/arroba-source/apps/kernel
COPY --chown=slice:slice apps/relay /opt/arroba-source/apps/relay

USER slice
RUN cargo build --manifest-path /opt/arroba-source/apps/kernel/Cargo.toml --bin arroba-kernel \
  && cargo build --manifest-path /opt/arroba-source/apps/relay/Cargo.toml --bin arroba-relay \
  && ln -sf /opt/arroba-source/apps/kernel/target/debug/arroba-kernel /opt/arroba-slice/bin/arroba-kernel \
  && ln -sf /opt/arroba-source/apps/relay/target/debug/arroba-relay /opt/arroba-slice/bin/arroba-relay

COPY --chown=slice:slice experiments/slice-spike/docker/browser-cdp.mjs /opt/arroba-slice/browser-cdp.mjs
COPY --chown=slice:slice experiments/slice-spike/docker/slice-screen.sh /opt/arroba-slice/slice-screen.sh
COPY --chown=slice:slice experiments/slice-spike/docker/start-providers.sh /opt/arroba-slice/start-providers.sh
COPY --chown=slice:slice experiments/slice-spike/docker/start-runtime.sh /opt/arroba-slice/start-runtime.sh
COPY --chown=slice:slice experiments/slice-spike/docker/validate-screen.sh /opt/arroba-slice/validate-screen.sh

RUN chmod +x /opt/arroba-slice/browser-cdp.mjs /opt/arroba-slice/slice-screen.sh /opt/arroba-slice/start-providers.sh /opt/arroba-slice/start-runtime.sh /opt/arroba-slice/validate-screen.sh

EXPOSE 43119 43130 43140 43252 6080
WORKDIR /workspace
CMD ["sleep", "infinity"]
