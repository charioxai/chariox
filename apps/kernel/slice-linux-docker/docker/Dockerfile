# syntax=docker/dockerfile:1
FROM node:26-bookworm-slim@sha256:367679cf9792759492a486e4aa4b421764d71a9546a6dae8aab81a99eb797b3e AS selkies-build

RUN apt-get update && apt-get install -y --no-install-recommends python3 python3-venv \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY apps/kernel/slice-linux-docker/fetch-selkies.mjs apps/kernel/slice-linux-docker/selkies.lock.json ./
COPY apps/kernel/slice-linux-docker/selkies-build/ ./locks/
ARG TARGETARCH
RUN bash /build/locks/build.sh "$TARGETARCH"

FROM debian:bookworm-slim AS selkies-runtime

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates python3 python3-venv \
    libdrm2 libegl1 libgbm1 libpixman-1-0 libpulse0 \
    libva2 libva-drm2 libva-x11-2 libwayland-server0 libxkbcommon0 \
    pulseaudio x11-utils x11-xserver-utils xvfb \
  && rm -rf /var/lib/apt/lists/*
RUN --mount=type=bind,from=selkies-build,source=/out,target=/tmp/selkies-package \
    --mount=type=bind,source=apps/kernel/slice-linux-docker/selkies-runtime.requirements.txt,target=/tmp/selkies-requirements.txt \
    python3 -m venv /opt/chariox-selkies \
  && /opt/chariox-selkies/bin/pip install --no-cache-dir --only-binary=:all: \
    --require-hashes --requirement /tmp/selkies-requirements.txt \
  && /opt/chariox-selkies/bin/pip install --no-cache-dir --no-index --no-deps \
    /tmp/selkies-package/selkies-*.whl /tmp/selkies-package/wheels/*.whl \
  && /opt/chariox-selkies/bin/pip check \
  && mkdir -p /usr/share/doc/chariox-selkies \
  && cp -a /tmp/selkies-package/licenses/. /usr/share/doc/chariox-selkies/ \
  && cp /tmp/selkies-requirements.txt /usr/share/doc/chariox-selkies/python-requirements.txt
ENV CHARIOX_SLICE_SELKIES_BIN=/opt/chariox-selkies/bin/selkies

FROM selkies-runtime AS slice-runtime-deps
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
    x11vnc \
    xclip \
    xdotool \
    zstd \
  && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
  && apt-get update \
  && apt-get install -y nodejs \
  && rm -rf /var/lib/apt/lists/*

FROM slice-runtime-deps AS slice-runtime
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
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-actions.mjs /opt/chariox-slice/browser-controller-actions.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-cdp.mjs /opt/chariox-slice/browser-controller-cdp.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-events.mjs /opt/chariox-slice/browser-controller-events.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-files.mjs /opt/chariox-slice/browser-controller-files.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-permissions.mjs /opt/chariox-slice/browser-controller-permissions.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller-snapshot.mjs /opt/chariox-slice/browser-controller-snapshot.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-controller.mjs /opt/chariox-slice/browser-controller.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/provider-port-bridge.mjs /opt/chariox-slice/provider-port-bridge.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/slice-screen.sh /opt/chariox-slice/slice-screen.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/slice-selkies.py /opt/chariox-slice/slice-selkies.py
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/start-providers.sh /opt/chariox-slice/start-providers.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/start-runtime.sh /opt/chariox-slice/start-runtime.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/validate-screen.sh /opt/chariox-slice/validate-screen.sh

RUN chmod +x /opt/chariox-slice/browser-cdp.mjs /opt/chariox-slice/browser-controller-actions.mjs /opt/chariox-slice/browser-controller-cdp.mjs /opt/chariox-slice/browser-controller-events.mjs /opt/chariox-slice/browser-controller-files.mjs /opt/chariox-slice/browser-controller-permissions.mjs /opt/chariox-slice/browser-controller-snapshot.mjs /opt/chariox-slice/browser-controller.mjs /opt/chariox-slice/provider-port-bridge.mjs /opt/chariox-slice/slice-screen.sh /opt/chariox-slice/start-providers.sh /opt/chariox-slice/start-runtime.sh /opt/chariox-slice/validate-screen.sh

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
