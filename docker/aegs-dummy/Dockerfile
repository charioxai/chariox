# syntax=docker/dockerfile:1

FROM rust:1.88-bookworm AS builder
ARG CHARIOX_BUILD_REVISION=development
ENV CHARIOX_BUILD_REVISION="${CHARIOX_BUILD_REVISION}"
WORKDIR /opt/chariox
COPY packages/event-protocol packages/event-protocol
COPY packages/aegs-sdk packages/aegs-sdk
COPY apps/aegs-dummy apps/aegs-dummy
WORKDIR /opt/chariox/apps/aegs-dummy
RUN cargo build --release --bin chariox-aegs-dummy \
  && cp target/release/chariox-aegs-dummy /tmp/chariox-aegs-dummy

FROM debian:bookworm-slim
ARG CHARIOX_BUILD_REVISION=development
LABEL org.opencontainers.image.title="chariox-aegs-dummy"
LABEL org.opencontainers.image.revision="${CHARIOX_BUILD_REVISION}"
LABEL dev.chariox.event-delivery-protocol-version="1"
ENV CHARIOX_BUILD_REVISION="${CHARIOX_BUILD_REVISION}"
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl tini \
  && rm -rf /var/lib/apt/lists/* \
  && useradd --create-home --uid 10001 --shell /usr/sbin/nologin chariox \
  && mkdir -p /var/lib/chariox/aegs \
  && chown -R chariox:chariox /var/lib/chariox
COPY --from=builder /tmp/chariox-aegs-dummy /usr/local/bin/chariox-aegs-dummy
USER chariox
WORKDIR /var/lib/chariox/aegs
ENTRYPOINT ["tini", "--", "/usr/local/bin/chariox-aegs-dummy"]
