# syntax=docker/dockerfile:1

FROM node:22-bookworm-slim

LABEL org.opencontainers.image.title="Arroba publication egress gateway"
LABEL org.opencontainers.image.description="Deny-by-default per-revision TLS CONNECT gateway."

RUN useradd --uid 10001 --no-create-home --shell /usr/sbin/nologin arroba-egress

WORKDIR /opt/arroba-egress
COPY docker/publication-egress/gateway.mjs ./gateway.mjs
COPY docker/publication-egress/policy.mjs ./policy.mjs
COPY docker/publication-egress/tls-client-hello.mjs ./tls-client-hello.mjs
RUN chown -R root:root /opt/arroba-egress \
  && chmod -R go-w /opt/arroba-egress

ENV HOST=0.0.0.0
ENV PORT=3128
EXPOSE 3128
USER 10001:10001
ENTRYPOINT ["node", "/opt/arroba-egress/gateway.mjs"]
