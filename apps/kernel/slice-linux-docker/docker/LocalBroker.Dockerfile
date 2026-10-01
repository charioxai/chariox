# Trusted local DEV broker. This image never contains a worker identity,
# credential profile, App publisher key or managed-release signature claim.
FROM node:22.17.1-bookworm@sha256:37ff334612f77d8f999c10af8797727b731629c26f2e83caa6af390998bdc49c AS ca-bundle
RUN echo 'a3413a37a8e09cc21b2c11c9ffb23d92d2fc9d1933c9e7617f5c4fba4f72d37d  /etc/ssl/certs/ca-certificates.crt' | sha256sum --check --strict

FROM node:22-bookworm-slim@sha256:d649c27dae7ba0137b3cef5dd75baa422c08dc3d9e3fc0c23dfb172dc3cc6436
COPY --from=ca-bundle /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN printf '%s\n' 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20260701T000000Z bookworm main' > /etc/apt/sources.list \
    && rm -f /etc/apt/sources.list.d/debian.sources \
    && apt-get -o Acquire::Check-Valid-Until=false update \
    && apt-get install -y --no-install-recommends docker.io bash ca-certificates coreutils util-linux tar zstd \
    && rm -rf /var/lib/apt/lists/*
ARG CHARIOX_LOCAL_SOURCE_DIGEST
LABEL org.chariox.local-broker.schema="1" org.chariox.local-broker.source="$CHARIOX_LOCAL_SOURCE_DIGEST"
USER 0:0
# Root installer mounts its verified immutable public source directory read-only.
# Private storage and the Unix transport directory are separate explicit mounts.
