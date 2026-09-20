# syntax=docker/dockerfile:1.7
FROM node:22.17.1-bookworm@sha256:37ff334612f77d8f999c10af8797727b731629c26f2e83caa6af390998bdc49c AS ca-bundle
RUN echo 'a3413a37a8e09cc21b2c11c9ffb23d92d2fc9d1933c9e7617f5c4fba4f72d37d  /etc/ssl/certs/ca-certificates.crt' | sha256sum --check --strict

FROM node:22-bookworm-slim@sha256:d649c27dae7ba0137b3cef5dd75baa422c08dc3d9e3fc0c23dfb172dc3cc6436 AS selkies-builder

ARG SELKIES_VERSION=0.0.0.dev0
ARG SELKIES_REVISION=3f87241fcd6abc44e205b22f6596e78ef4946670
ARG SELKIES_SOURCE_SHA256=9aee9dcaf6e6617194b84a667fd7fe441b2cf85718abf63e109f29db18c90d51
ARG TARGETARCH
ENV DEBIAN_FRONTEND=noninteractive
RUN printf '%s\n' 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20260701T000000Z bookworm main' > /etc/apt/sources.list \
  && rm -f /etc/apt/sources.list.d/debian.sources \
  && apt-get -o Acquire::Check-Valid-Until=false update \
  && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    python3 \
    python3-pip \
    python3-venv \
  && rm -rf /var/lib/apt/lists/*
WORKDIR /tmp/chariox-selkies-build
COPY apps/kernel/slice-linux-docker/docker/selkies-web-core.package-lock.json /tmp/selkies-web-core.package-lock.json
COPY apps/kernel/slice-linux-docker/docker/selkies-dashboard.package-lock.json /tmp/selkies-dashboard.package-lock.json
RUN curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
      "https://github.com/selkies-project/selkies/archive/$SELKIES_REVISION.tar.gz" \
      -o selkies.tar.gz \
  && printf '%s  %s\n' "$SELKIES_SOURCE_SHA256" selkies.tar.gz \
      | sha256sum --check --strict \
  && mkdir source \
  && tar -xzf selkies.tar.gz --strip-components=1 -C source
RUN cd source/addons/selkies-web-core \
  && cp /tmp/selkies-web-core.package-lock.json package-lock.json \
  && npm ci --no-audit --no-fund \
  && npm run build \
  && cd ../selkies-dashboard \
  && cp /tmp/selkies-dashboard.package-lock.json package-lock.json \
  && npm ci --no-audit --no-fund \
  && SELKIES_INJECT=1 npm run build \
  && mkdir -p ../../src/selkies/selkies_web \
  && cp ../selkies-web-core/dist/selkies-core.js dist/src/ \
  && cp ../universal-touch-gamepad/universalTouchGamepad.js dist/src/ \
  && cp -a dist/. ../../src/selkies/selkies_web/ \
  && printf '%s\n' '"""Bundled Selkies web client."""' > ../../src/selkies/selkies_web/__init__.py \
  && printf '%s\n' '{"name":"Selkies","short_name":"Selkies","display":"fullscreen","background_color":"#000000","theme_color":"#000000","icons":[{"src":"icon-512.png","type":"image/png","sizes":"512x512"}],"start_url":"."}' > ../../src/selkies/selkies_web/manifest.json \
  && cp ../../docs/assets/logo/icon-512x512.png ../../src/selkies/selkies_web/icon-512.png \
  && cp ../../docs/assets/logo/favicon.ico ../../src/selkies/selkies_web/favicon.ico
RUN python3 -m venv /tmp/chariox-selkies-build-venv \
  && /tmp/chariox-selkies-build-venv/bin/pip install --no-cache-dir \
    build==1.3.0 setuptools==80.9.0 wheel==0.45.1 \
  && cd source \
  && /tmp/chariox-selkies-build-venv/bin/python -m build --wheel --no-isolation --outdir /out/selkies-wheel \
  && mkdir -p /out/runtime-wheels \
  && cp LICENSE /out/selkies-LICENSE.txt \
  && printf '%s\n' \
    "package=selkies" \
    "version=$SELKIES_VERSION" \
    "source_revision=$SELKIES_REVISION" \
    "source_url=https://github.com/selkies-project/selkies/commit/$SELKIES_REVISION" \
    "source_archive_sha256=$SELKIES_SOURCE_SHA256" \
    'license=MPL-2.0' \
    'license_url=https://mozilla.org/MPL/2.0/' \
    'pixelflux_version=2.1.0' \
    'pixelflux_source_revision=6974a8c16a14dc0040d5ed937d633a6ceb4c926d' \
    'pixelflux_source_url=https://github.com/selkies-project/pixelflux/commit/6974a8c16a14dc0040d5ed937d633a6ceb4c926d' \
    'pixelflux_license=MPL-2.0 (bundled GPL/LGPL codec notices retained upstream)' \
    'pcmflux_version=2.1.0' \
    'pcmflux_source_revision=a8fac8d8ea89117a1c2480bdbefbb3b8b717d956' \
    'pcmflux_source_url=https://github.com/selkies-project/pcmflux/commit/a8fac8d8ea89117a1c2480bdbefbb3b8b717d956' \
    'pcmflux_license=MPL-2.0 (bundled LGPL audio notices retained upstream)' \
    > /out/selkies-source-metadata.txt
RUN architecture="${TARGETARCH:-amd64}" \
  && case "$architecture" in \
    amd64) \
      pixelflux_url='https://github.com/selkies-project/pixelflux/releases/download/6974a8c/pixelflux-2.1.0-cp311-cp311-manylinux_2_28_x86_64.whl' \
      pixelflux_filename='pixelflux-2.1.0-cp311-cp311-manylinux_2_28_x86_64.whl' \
      pixelflux_sha256='97a1b6b289373373b8a32699bade666d758848be11fda470f5e49a5d50512756' \
      pcmflux_url='https://github.com/selkies-project/pcmflux/releases/download/a8fac8d/pcmflux-2.1.0-cp311-cp311-manylinux_2_28_x86_64.whl' \
      pcmflux_filename='pcmflux-2.1.0-cp311-cp311-manylinux_2_28_x86_64.whl' \
      pcmflux_sha256='5fc902b41888eaf8185f891709c995198152fd7febfd158d130cba18bd07508c' ;; \
    arm64) \
      pixelflux_url='https://github.com/selkies-project/pixelflux/releases/download/6974a8c/pixelflux-2.1.0-cp311-cp311-manylinux_2_28_aarch64.whl' \
      pixelflux_filename='pixelflux-2.1.0-cp311-cp311-manylinux_2_28_aarch64.whl' \
      pixelflux_sha256='b86783c2fb7eb14b105f123b8182be7144afb9bd4d2c2af181b5731b5e966342' \
      pcmflux_url='https://github.com/selkies-project/pcmflux/releases/download/a8fac8d/pcmflux-2.1.0-cp311-cp311-manylinux_2_28_aarch64.whl' \
      pcmflux_filename='pcmflux-2.1.0-cp311-cp311-manylinux_2_28_aarch64.whl' \
      pcmflux_sha256='d34fb03f25ad0a977bbcb19d8aee061b5125b8ee8be96be2a8398bd6097681ef' ;; \
    *) echo "unsupported Selkies architecture: $architecture" >&2; exit 1 ;; \
  esac \
  && curl --fail --silent --show-error --location --proto '=https' "$pixelflux_url" -o "/out/runtime-wheels/$pixelflux_filename" \
  && printf '%s  %s\n' "$pixelflux_sha256" "/out/runtime-wheels/$pixelflux_filename" | sha256sum --check --strict \
  && curl --fail --silent --show-error --location --proto '=https' "$pcmflux_url" -o "/out/runtime-wheels/$pcmflux_filename" \
  && printf '%s  %s\n' "$pcmflux_sha256" "/out/runtime-wheels/$pcmflux_filename" | sha256sum --check --strict

FROM rust:1.88-bookworm@sha256:af306cfa71d987911a781c37b59d7d67d934f49684058f96cf72079c3626bfe0 AS rust-builder
WORKDIR /opt/chariox-source
COPY --from=ca-bundle /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN printf '%s\n' 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20260701T000000Z bookworm main' > /etc/apt/sources.list \
  && rm -f /etc/apt/sources.list.d/debian.sources \
  && apt-get -o Acquire::Check-Valid-Until=false update \
  && apt-get install -y --no-install-recommends \
    libdbus-1-dev \
    libssl-dev \
    pkg-config \
    protobuf-compiler \
  && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY adapters/rust adapters/rust
COPY apps/aegs-dummy apps/aegs-dummy
COPY apps/kernel apps/kernel
COPY apps/relay apps/relay
COPY examples/workflow-code examples/workflow-code
COPY packages/aegs-sdk packages/aegs-sdk
COPY packages/event-protocol packages/event-protocol
COPY apps/kernel/slice-linux-docker/prebuilt /opt/chariox-prebuilt
ARG CHARIOX_PREBUILT_RUNTIME=0
RUN if [ "$CHARIOX_PREBUILT_RUNTIME" = "1" ]; then \
      test -x /opt/chariox-prebuilt/chariox-kernel \
      && test -x /opt/chariox-prebuilt/chariox-relay \
      && mkdir -p target/release \
      && install -m 0755 /opt/chariox-prebuilt/chariox-kernel target/release/chariox-kernel \
      && install -m 0755 /opt/chariox-prebuilt/chariox-relay target/release/chariox-relay; \
    else \
      CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo build --locked --release --package chariox-kernel --bin chariox-kernel --bin chariox-managed-bootstrap \
      && CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 cargo build --locked --release --package chariox-relay --bin chariox-relay; \
    fi

FROM node:22-bookworm-slim@sha256:d649c27dae7ba0137b3cef5dd75baa422c08dc3d9e3fc0c23dfb172dc3cc6436

ENV DEBIAN_FRONTEND=noninteractive
ENV CHARIOX_SLICE_ROOT=/opt/chariox-slice
ENV CHARIOX_SLICE_SELKIES_BIN=/opt/chariox-selkies/bin/selkies

COPY --from=ca-bundle /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN printf '%s\n' 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20260701T000000Z bookworm main' > /etc/apt/sources.list \
  && rm -f /etc/apt/sources.list.d/debian.sources \
  && apt-get -o Acquire::Check-Valid-Until=false update \
  && apt-get install -y --no-install-recommends \
    bash \
    bubblewrap \
    build-essential \
    ca-certificates \
    chromium \
    chromium-driver \
    chromium-sandbox \
    curl \
    fonts-dejavu \
    fonts-liberation \
    gh \
    git \
    jq \
    libdbus-1-dev \
    libdrm2 \
    libegl1 \
    libgbm1 \
    libssl-dev \
    libpixman-1-0 \
    libpulse0 \
    libva-drm2 \
    libva2 \
    libva-x11-2 \
    libwayland-server0 \
    lsof \
    novnc \
    openbox \
    pkg-config \
    procps \
    protobuf-compiler \
    python3 \
    python3-venv \
    screen \
    scrot \
    socat \
    tesseract-ocr \
    tesseract-ocr-eng \
    unzip \
    websockify \
    x11-utils \
    x11vnc \
    xclip \
    xdotool \
    xvfb \
    zstd \
  && rm -rf /var/lib/apt/lists/*

COPY --from=selkies-builder /out/selkies-wheel/ /tmp/selkies-wheel/
COPY --from=selkies-builder /out/runtime-wheels/ /tmp/selkies-runtime-wheels/
COPY --from=selkies-builder /out/selkies-LICENSE.txt /tmp/selkies-LICENSE.txt
COPY --from=selkies-builder /out/selkies-source-metadata.txt /tmp/selkies-source-metadata.txt

RUN <<'EOF'
set -eu
cat > /tmp/chariox-selkies-requirements.txt <<'REQUIREMENTS'
# Python 3.11 runtime wheels for Debian amd64 and arm64.
# Hashes are from exact-version PyPI release metadata; source builds are forbidden.
aiofiles==25.1.0 \
    --hash=sha256:abe311e527c862958650f9438e859c1fa7568a141b22abcd015e120e86a85695
aiohappyeyeballs==2.7.1 \
    --hash=sha256:9243213661e29250eb41368e5daa826fc017156c3b8a11440826b2e3ed376472
aiohttp==3.14.3 \
    --hash=sha256:d6088ec9894113802bddb3c09e974929aed2c7b3a8c456219b8aab4481f1a239 \
    --hash=sha256:53e7b4ce82b54a8bcc71b3b67a5cbd177ca1d7f592cbc92cd38b7349f73482db
aiosignal==1.4.0 \
    --hash=sha256:053243f8b92b990551949e63930a839ff0cf0b0ebbe0597b0f3fb19e1a0fe82e
aitop==0.16.10 \
    --hash=sha256:b9a2d7ff31c1c8bb1b23f8ffd126edbbd6bdab2931ad8c031bbbac60f59e48de
attrs==26.1.0 \
    --hash=sha256:c647aa4a12dfbad9333ca4e71fe62ddc36f4e63b2d260a37a8b83d2f043ac309
cffi==2.1.1 \
    --hash=sha256:3311ed60d36f83378794e1009ac6258bafbf81f7888b4caa7b35a521e3f95813 \
    --hash=sha256:34e261f78cb6ceaaa36f42f2613f4380d94d9c759a9c73c769ee6e0247364632
cryptography==50.0.1 \
    --hash=sha256:e74591e283fe6eb956416c929eb58262a719fe0311fd9054c62c3350ed8760d8 \
    --hash=sha256:51593d180cf6d179bde5c5d065bed81386b1f381656ae7d042b7ffc87a9895ad \
    --hash=sha256:e2ca8fd1b6b4b82a1c4cb02841d0837e3c12336c2e24b520ab8ab3b969733d8f \
    --hash=sha256:51afcfceb15597cf2635068e4ac9a56b2abde622edde17f37d85fd7b5306497a \
    --hash=sha256:53e279950892dc102c6b4e52af03ae5ea92fac572a1ddab78ca73a997f62b69f \
    --hash=sha256:ff838d62ec1bfce4f9ba7fa16f4a7b554cd8d0c299e6be37502161a660c84eef \
    --hash=sha256:4c4188f7c0cf655be5c06342b817ed0f9595b69ffa2b12026e5353eed29dea88 \
    --hash=sha256:407fe2b6db00939c05c0e945e9914238f2f0a430974839429dafc82b1ee6bee5 \
    --hash=sha256:01f41478cf33fc605a6a089cd56d28b45c6c0b45a1928b61797f2621a04bac71 \
    --hash=sha256:9dde0a357190eb3b1da1bb9ab750e9c85cba82ca5977aa0836cbb94e92611239 \
    --hash=sha256:05ba322c4da95b262a212c345af888ef2c37c88c0509756ea00a0e6d68850f23 \
    --hash=sha256:e22dfed744bd4002e909464cb23d2f0b05c6f3113a79ef2e9864a53db737c733
dnspython==2.8.0 \
    --hash=sha256:01d9bbc4a2d76bf0db7c1f729812ded6d912bd318d3b1cf81d30c0f845dbf3af
frozenlist==1.8.0 \
    --hash=sha256:2552f44204b744fba866e573be4c1f9048d6a324dfe14475103fd51613eb1d1f \
    --hash=sha256:957e7c38f250991e48a9a73e6423db1bb9dd14e722a10f6b8bb8e16a0f55f695 \
    --hash=sha256:0c18a16eab41e82c295618a77502e17b195883241c563b00f0aa5106fc4eaa0d
google-crc32c==1.8.0 \
    --hash=sha256:19b40d637a54cb71e0829179f6cb41835f0fbd9e8eb60552152a8b52c36cbe15 \
    --hash=sha256:17446feb05abddc187e5441a45971b8394ea4c1b6efd88ab0af393fd9e0a156a
idna==3.19 \
    --hash=sha256:815e7be7a7806d54abb586dc943addc79e8b2ee16915059658cbeff4b1b43bf4
ifaddr==0.2.0 \
    --hash=sha256:085e0305cfe6f16ab12d72e2024030f5d52674afad6911bb1eee207177b8a748
msgpack==1.2.2 \
    --hash=sha256:b9b0c1f2aa7b0026b4bd50718100e8b04175e4f36e160aa852502377b5e572e7 \
    --hash=sha256:f11e09f10210a91c169e39c7a5a1f9090eaa73ad75555fafad5023c3053c47ba
multidict==6.7.1 \
    --hash=sha256:2e2d2ed645ea29f31c4c7ea1552fcfd7cb7ba656e1eafd4134a6620c9f5fdd9e \
    --hash=sha256:439cbebd499f92e9aa6793016a8acaa161dfa749ae86d20960189f5398a19144 \
    --hash=sha256:55d97cc6dae627efa6a6e548885712d4864b81110ac76fa4e534c03819fa4a56
nvidia-ml-py==13.610.43 \
    --hash=sha256:f13c72698edef492f985cc225f14faafe68ae065a2e407f45bdf6f4b9b43fde8
pillow==12.3.0 \
    --hash=sha256:bcb46e2f9feff8d06323983bd83ed00c201fdcab3d74973e7072a889b3979fcd \
    --hash=sha256:23d27a3e0307ec2244cc51e7287b919aa68d097504ebe19df4e76a98a3eea5bd
prometheus_client==0.26.0 \
    --hash=sha256:fa93d06737aa02bacd05794768508bb97d2fbee28cb3bca04eaae92f0ca953d6
propcache==0.5.2 \
    --hash=sha256:2293949b855ce597f2826452d17c2d545fb5622379c4ea6fdf525e9b8e8a2511 \
    --hash=sha256:5aaa2b923c1944ac8febd6609cb373540a5563e7cbcb0fd770f75dace2eb817b \
    --hash=sha256:be1ddfcbb376e3de5d2e2db1d58d6d67463e6b4f9f040c000de8e300295465fe
psutil==6.1.1 \
    --hash=sha256:97f7cb9921fbec4904f522d972f0c0e1f4fabbdd4e0287813b21215074a0f160 \
    --hash=sha256:33431e84fee02bc84ea36d9e2c4a6d395d479c9dd9bba2376c1f6ee8f3a4e0b3
pulsectl==24.12.0 \
    --hash=sha256:13a60be940594f03ead3245b3dfe3aff4a3f9a792af347674bde5e716d4f76d2
pulsectl-asyncio==1.3.2 \
    --hash=sha256:aa3adf89fe91a80ec1a5c92de3dcfd17a8f171006a07a2ec59a4a13d031fc70d
pycparser==3.0 \
    --hash=sha256:b727414169a36b7d524c1c3e31839a521725078d7b2ff038656844266160a992
pyee==13.0.1 \
    --hash=sha256:af2f8fede4171ef667dfded53f96e2ed0d6e6bd7ee3bb46437f77e3b57689228
pylibsrtp==1.0.0 \
    --hash=sha256:9209b86e662ebbd17c8a9e8549ba57eca92a3e87fb5ba8c0e27b8c43cd08a767 \
    --hash=sha256:293c9f2ac21a2bd689c477603a1aa235d85cf252160e6715f0101e42a43cbedc
pyOpenSSL==26.4.0 \
    --hash=sha256:f0eb0cb2d581d3ad2b9c489468485e7f2ab6727d08401bcf9d824c3caddf3c1c
typing_extensions==4.16.0 \
    --hash=sha256:481caa481374e813c1b176ada14e97f1f67a4539ce9cfeb3f350d78d6370c2e8
uvloop==0.22.1 \
    --hash=sha256:53c85520781d84a4b8b230e24a5af5b0778efdb39142b424990ff1ef7c48ba21 \
    --hash=sha256:56a2d1fae65fd82197cb8c53c367310b3eabe1bbb9fb5a04d28e3e3520e4f702
watchdog==6.0.0 \
    --hash=sha256:7607498efa04a3542ae3e05e64da8202e58159aa1fa4acddf7678d34a35d4f13 \
    --hash=sha256:20ffe5b202af80ab4266dcd3e91aae72bf2da48c0d33bdb15c66658e685e94e2
yarl==1.24.5 \
    --hash=sha256:874019bd513008b009f58657134e5d0c5e030b3559bd0553976837adf52fe966 \
    --hash=sha256:570fec8fbd22b032733625f03f10b7ff023bc399213db15e72a7acaef28c2f4e \
    --hash=sha256:a33700d13d9b7d84fd10947b09ff69fb9a792e519c8cb9764a3ca70baa6c23a7
REQUIREMENTS
EOF
ARG CHARIOX_SELKIES_VERSION=0.0.0.dev0
ARG CHARIOX_SELKIES_REVISION=3f87241fcd6abc44e205b22f6596e78ef4946670
RUN python3 -m venv /opt/chariox-selkies \
  && /opt/chariox-selkies/bin/pip install --no-cache-dir --only-binary=:all: --require-hashes \
    --requirement /tmp/chariox-selkies-requirements.txt \
  && /opt/chariox-selkies/bin/pip install --no-cache-dir --only-binary=:all: --no-index --no-deps \
    /tmp/selkies-runtime-wheels/*.whl \
  && /opt/chariox-selkies/bin/pip install --no-cache-dir --only-binary=:all: --no-index --no-deps \
    /tmp/selkies-wheel/*.whl \
  && /opt/chariox-selkies/bin/pip check \
  && test "$(/opt/chariox-selkies/bin/selkies --version)" = "selkies $CHARIOX_SELKIES_VERSION" \
  && install -d -o root -g root -m 0755 /usr/share/doc/chariox-selkies \
  && install -o root -g root -m 0644 /tmp/selkies-LICENSE.txt /usr/share/doc/chariox-selkies/LICENSE \
  && install -o root -g root -m 0644 /tmp/selkies-source-metadata.txt /usr/share/doc/chariox-selkies/SOURCE-METADATA \
  && rm -rf /tmp/chariox-selkies-requirements.txt /tmp/selkies-wheel /tmp/selkies-runtime-wheels /tmp/selkies-LICENSE.txt /tmp/selkies-source-metadata.txt

WORKDIR /opt/chariox-toolchain
COPY apps/kernel/slice-linux-docker/toolchain/package.json apps/kernel/slice-linux-docker/toolchain/package-lock.json ./
RUN npm_config_cache=/tmp/chariox-npm-cache npm ci --omit=dev \
  && test "$(node_modules/.bin/codex --version)" = "codex-cli 0.144.0" \
  && test "$(node_modules/.bin/opencode --version)" = "1.18.23" \
  && test "$(node_modules/.bin/claude --version)" = "2.1.212 (Claude Code)" \
  && test "$(node_modules/.bin/pnpm --version)" = "11.22.0" \
  && rm -rf /tmp/chariox-npm-cache /root/.npm
ENV PATH=/opt/chariox-toolchain/node_modules/.bin:/usr/local/bin:/usr/local/sbin:/usr/sbin:/usr/bin:/sbin:/bin

RUN groupadd --gid 1001 slice \
  && useradd --uid 1001 --gid 1001 -m -s /bin/bash slice \
  && mkdir -p /opt/chariox-slice/bin /opt/chariox-slice/logs \
  && chown -R slice:slice /opt/chariox-slice

COPY --from=rust-builder --chown=slice:slice /opt/chariox-source/target/release/chariox-kernel /opt/chariox-slice/bin/chariox-kernel
COPY --from=rust-builder --chown=slice:slice /opt/chariox-source/target/release/chariox-relay /opt/chariox-slice/bin/chariox-relay
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/browser-cdp.mjs /opt/chariox-slice/browser-cdp.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe.mjs /opt/chariox-slice/managed-provider-isolation-probe.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/managed-provider-isolation-probe-wrapper.sh /opt/chariox-slice/managed-provider-isolation-probe-wrapper.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/provider-port-bridge.mjs /opt/chariox-slice/provider-port-bridge.mjs
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/slice-screen.sh /opt/chariox-slice/slice-screen.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/start-providers.sh /opt/chariox-slice/start-providers.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/start-runtime.sh /opt/chariox-slice/start-runtime.sh
COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/validate-screen.sh /opt/chariox-slice/validate-screen.sh

RUN chmod +x /opt/chariox-slice/browser-cdp.mjs /opt/chariox-slice/managed-provider-isolation-probe.mjs /opt/chariox-slice/managed-provider-isolation-probe-wrapper.sh /opt/chariox-slice/provider-port-bridge.mjs /opt/chariox-slice/slice-screen.sh /opt/chariox-slice/start-providers.sh /opt/chariox-slice/start-runtime.sh /opt/chariox-slice/validate-screen.sh

ARG CHARIOX_RELAY_PEER_PROTOCOL_VERSION=unknown
ARG CHARIOX_RUNTIME_SOURCE_REVISION=unknown
LABEL io.chariox.relay-peer-protocol-version="${CHARIOX_RELAY_PEER_PROTOCOL_VERSION}"
LABEL io.chariox.runtime-source-revision="${CHARIOX_RUNTIME_SOURCE_REVISION}"
LABEL io.chariox.debian-snapshot="20260701T000000Z"
LABEL io.chariox.selkies-version="${CHARIOX_SELKIES_VERSION}"
LABEL io.chariox.selkies-source-revision="${CHARIOX_SELKIES_REVISION}"
LABEL io.chariox.selkies-license="MPL-2.0"
LABEL io.chariox.selkies-source="https://github.com/selkies-project/selkies/commit/${CHARIOX_SELKIES_REVISION}"
LABEL org.opencontainers.image.licenses="MPL-2.0"

RUN printf '%s\n' "${CHARIOX_RUNTIME_SOURCE_REVISION}" > /opt/chariox-slice/runtime-source-revision

USER slice
EXPOSE 43119 43130 43140 43252 6080
WORKDIR /workspace
CMD ["sleep", "infinity"]
