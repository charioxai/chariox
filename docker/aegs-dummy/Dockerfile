# syntax=docker/dockerfile:1

FROM rust:1.88-bookworm AS builder
ARG ARROBA_BUILD_REVISION=development
ENV ARROBA_BUILD_REVISION="${ARROBA_BUILD_REVISION}"
WORKDIR /opt/arroba
COPY packages/event-protocol packages/event-protocol
COPY packages/aegs-sdk packages/aegs-sdk
COPY apps/aegs-dummy apps/aegs-dummy
WORKDIR /opt/arroba/apps/aegs-dummy
RUN cargo build --release --bin arroba-aegs-dummy \
  && cp target/release/arroba-aegs-dummy /tmp/arroba-aegs-dummy

FROM debian:bookworm-slim
ARG ARROBA_BUILD_REVISION=development
LABEL org.opencontainers.image.title="arroba-aegs-dummy"
LABEL org.opencontainers.image.revision="${ARROBA_BUILD_REVISION}"
LABEL dev.arroba.event-delivery-protocol-version="1"
ENV ARROBA_BUILD_REVISION="${ARROBA_BUILD_REVISION}"
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates curl tini \
  && rm -rf /var/lib/apt/lists/* \
  && useradd --create-home --uid 10001 --shell /usr/sbin/nologin arroba \
  && mkdir -p /var/lib/arroba/aegs \
  && chown -R arroba:arroba /var/lib/arroba
COPY --from=builder /tmp/arroba-aegs-dummy /usr/local/bin/arroba-aegs-dummy
USER arroba
WORKDIR /var/lib/arroba/aegs
ENTRYPOINT ["tini", "--", "/usr/local/bin/arroba-aegs-dummy"]
