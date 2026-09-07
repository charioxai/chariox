FROM node:22.17.1-bookworm@sha256:37ff334612f77d8f999c10af8797727b731629c26f2e83caa6af390998bdc49c AS ca-bundle
RUN echo 'a3413a37a8e09cc21b2c11c9ffb23d92d2fc9d1933c9e7617f5c4fba4f72d37d  /etc/ssl/certs/ca-certificates.crt' | sha256sum --check --strict

# Intentionally omits the kernel and provider toolchains. prepare.mjs verifies
# these immutable browser inputs against the production slice Dockerfile.
FROM node:22-bookworm-slim@sha256:d649c27dae7ba0137b3cef5dd75baa422c08dc3d9e3fc0c23dfb172dc3cc6436
ARG CHARIOX_DRILL_ID
LABEL io.chariox.chromium-drill="${CHARIOX_DRILL_ID}"
ENV DEBIAN_FRONTEND=noninteractive CHARIOX_SLICE_ROOT=/opt/chariox-slice HOME=/home/slice
COPY --from=ca-bundle /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN printf '%s\n' 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20260701T000000Z bookworm main' > /etc/apt/sources.list \
  && rm -f /etc/apt/sources.list.d/debian.sources \
  && apt-get -o Acquire::Check-Valid-Until=false update \
  && apt-get install -y --no-install-recommends bash chromium chromium-sandbox curl fonts-dejavu fonts-liberation novnc openbox procps python3 websockify x11-utils x11vnc xdotool xvfb zstd \
  && rm -rf /var/lib/apt/lists/* \
  && groupadd --gid 1001 slice \
  && useradd --uid 1001 --gid 1001 -m -s /bin/bash slice \
  && mkdir -p /opt/chariox-slice/logs /opt/chariox-drill \
  && chown -R slice:slice /opt/chariox-slice
COPY slice-screen.sh browser-cdp.mjs chromium-sandbox-probe.mjs /opt/chariox-slice/
COPY fixture-server.mjs cdp.mjs profile.mjs /opt/chariox-drill/
RUN chmod 0755 /opt/chariox-slice/*.sh /opt/chariox-slice/*.mjs \
  && chmod -R a+rX /opt/chariox-drill
USER slice
WORKDIR /home/slice
CMD ["node", "/opt/chariox-drill/fixture-server.mjs"]
