# syntax=docker/dockerfile:1

FROM node:22-bookworm-slim@sha256:d649c27dae7ba0137b3cef5dd75baa422c08dc3d9e3fc0c23dfb172dc3cc6436

LABEL org.opencontainers.image.title="Chariox publication egress gateway"
LABEL org.opencontainers.image.description="Deny-by-default per-revision TLS CONNECT gateway."

RUN useradd --uid 10001 --no-create-home --shell /usr/sbin/nologin chariox-egress

WORKDIR /opt/chariox-egress
COPY docker/publication-egress/gateway.mjs ./gateway.mjs
COPY docker/publication-egress/policy.mjs ./policy.mjs
COPY docker/publication-egress/tls-client-hello.mjs ./tls-client-hello.mjs
RUN chown -R root:root /opt/chariox-egress \
  && chmod -R a+rX,go-w /opt/chariox-egress

ENV HOST=0.0.0.0
ENV PORT=3128
EXPOSE 3128
USER 10001:10001
ENTRYPOINT ["node", "/opt/chariox-egress/gateway.mjs"]
